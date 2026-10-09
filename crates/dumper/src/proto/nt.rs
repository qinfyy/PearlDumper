use std::{
    collections::{BTreeSet, HashMap, HashSet},
    time::{Duration, Instant},
};

use anyhow::Context;
use iced_x86::{Decoder, DecoderOptions, FlowControl, Register};
use il2cpp::{FUNCTIONS_TABLE_REFLECTION, GA_BASE, vm::class::Il2CppClass};
use reflection::runtime_type::RuntimeType;
use utils::scanner::{
    game_assembly_code_sections, game_assembly_function, game_assembly_functions,
};

use super::{
    MessageMinimalInfo, native,
    recovery::{Evidence, Hints, response, response_names},
    runtime::{PacketRegistry, ProtocolRuntime},
    type_references::TypeReferences,
};

pub(super) struct RequestScan {
    runtime: ProtocolRuntime,
    references: TypeReferences,
    targets: HashMap<usize, native::SendRegisters>,
    method_names: HashMap<usize, BTreeSet<String>>,
    pub registry: PacketRegistry,
    pub name_hints: Hints,
}

pub(super) fn prepare_request_scan() -> anyhow::Result<RequestScan> {
    let references = TypeReferences::from_runtime()?;
    let mut runtime = ProtocolRuntime::new()?;
    let targets = runtime.send_targets()?;
    let registry = runtime.packet_registry(&references)?;
    let method_names = runtime.xlua_method_names()?;
    Ok(RequestScan {
        runtime,
        references,
        targets,
        method_names,
        registry,
        name_hints: Hints::new(),
    })
}

pub(super) fn get_req_map(
    scan: &mut RequestScan,
    minimal_info: &HashMap<RuntimeType, MessageMinimalInfo>,
    req_map: &mut HashMap<RuntimeType, (u16, Option<String>)>,
) -> anyhow::Result<HashMap<RuntimeType, Vec<usize>>> {
    let mut request_classes = HashMap::new();
    // The native registries own coverage. A missing send call only loses a name or RVA,
    // never the packet itself, including requests used solely through generic code.
    for (&ty, &id) in &scan.registry.all {
        anyhow::ensure!(
            minimal_info.contains_key(&ty),
            "registered CmdId {id} has no protobuf definition: {}",
            ty.il_name()
        );
        if !scan.registry.responses.contains_key(&ty) {
            req_map.insert(ty, (id, None));
            request_classes.insert(ty.get_il2cpp_type().get_class(), ty);
        }
    }

    let started = Instant::now();
    log::info!("Locating send references using the PE exception directory");
    let functions = native::reference_functions(
        game_assembly_code_sections(),
        *GA_BASE,
        &scan.targets,
        game_assembly_functions()?,
    );
    log::info!(
        "Analyzing {} native functions for {} registered requests",
        functions.len(),
        req_map.len()
    );
    let mut last_progress = Instant::now();
    let mut names: HashMap<RuntimeType, BTreeSet<String>> = HashMap::new();
    let mut rvas: HashMap<RuntimeType, Vec<usize>> = HashMap::new();
    let mut rejected = 0;
    let mut undecodable = 0;
    for (index, rva) in functions.iter().copied().enumerate() {
        if last_progress.elapsed() >= Duration::from_secs(2) {
            log::info!(
                "Request names: {index}/{} functions, {} packet handlers, {:.1}s elapsed",
                functions.len(),
                rvas.len(),
                started.elapsed().as_secs_f64()
            );
            last_progress = Instant::now();
        }
        let bytes = game_assembly_function(rva)?;
        scan.runtime.initialize_references(rva, bytes)?;
        let analysis = match native::scan(
            bytes,
            *GA_BASE + rva,
            scan.runtime.allocation,
            &HashSet::new(),
            &scan.targets,
        ) {
            Ok(analysis) => analysis,
            Err(error) => {
                undecodable += 1;
                log::debug!("Cannot analyze optional request name at RVA 0x{rva:X}: {error:#}");
                continue;
            }
        };
        for entry in analysis.requests {
            let ty = scan
                .references
                .class_from_slot(entry.class_slot)
                .and_then(|class| request_classes.get(&class))
                .copied();
            let Some(ty) = ty.filter(|ty| req_map[ty].0 == entry.id) else {
                rejected += 1;
                continue;
            };
            rvas.entry(ty).or_default().push(rva);
            if let Some(aliases) = scan.method_names.get(&(*GA_BASE + rva)) {
                for name in aliases {
                    names.entry(ty).or_default().insert(name.clone());
                    scan.name_hints
                        .context(format!("XLua::{name}; CmdId={}", entry.id), rva);
                    scan.name_hints
                        .insert(ty.format_type_name(true), name.clone());
                }
            }
        }
    }
    let mut conflicts = 0;
    for (ty, candidates) in names {
        if candidates.len() == 1 {
            req_map.get_mut(&ty).unwrap().1 = candidates.into_iter().next();
        } else {
            conflicts += 1;
            let identity = ty.format_type_name(true);
            for c in &mut scan.name_hints.entries {
                if c.symbol == identity {
                    c.evidence = Evidence::Hint;
                    c.source.push_str(" (shared request business alias)");
                }
            }
        }
    }
    for entries in rvas.values_mut() {
        entries.sort_unstable();
        entries.dedup();
    }
    log::info!(
        "Requests: {} IDs, {} names, {} types with handlers, {conflicts} name conflicts, {rejected} unverified calls, {undecodable} undecodable functions ({:.2}s)",
        req_map.len(),
        req_map.values().filter(|(_, name)| name.is_some()).count(),
        rvas.len(),
        started.elapsed().as_secs_f64()
    );
    Ok(rvas)
}

pub(super) fn get_rsp_notify_names(
    scan: &mut RequestScan,
) -> anyhow::Result<HashMap<String, String>> {
    let methods = FUNCTIONS_TABLE_REFLECTION
        .get()
        .context("native method cache is missing")?;
    let prefixes = ["_OnCmd", "_Cmd", "_On", "OnCmd", "On", "Cmd"];
    let mut by_rva: HashMap<usize, HashSet<RuntimeType>> = HashMap::new();
    for (&ty, rvas) in &scan.registry.handlers {
        for &rva in rvas {
            by_rva.entry(rva).or_default().insert(ty);
        }
    }
    let response_classes: HashMap<Il2CppClass, RuntimeType> = scan
        .registry
        .responses
        .keys()
        .map(|&ty| (ty.get_il2cpp_type().get_class(), ty))
        .collect();
    let strings = ResponseStrings::new();
    let loggers = [
        ("RPG.SuperDebug::LogError(System.String)", Register::RCX),
        (
            "RPG.SuperDebug::LogError(RPG.LogTag,System.String)",
            Register::RDX,
        ),
    ]
    .into_iter()
    .filter_map(|(signature, argument)| {
        let method = methods.get(signature)?;
        Some((*GA_BASE + method.checked_rva().ok()?, argument))
    })
    .collect();
    let mut hints = Hints::new();
    let mut helper_cache = HashMap::new();
    for (signature, method) in methods {
        if !signature.ends_with("(System.UInt16,System.Object)") {
            continue;
        }
        if !reflection::method_info::MethodInfo::from_handle(*method).is_ok_and(|m| !m.is_static())
        {
            continue;
        }
        let method_name = method.get_name();
        let Some(name) = prefixes
            .iter()
            .find_map(|prefix| method_name.strip_prefix(prefix))
        else {
            continue;
        };
        if name.is_empty() || super::util::is_obf(name) {
            continue;
        }
        let Ok(rva) = method.checked_rva() else {
            continue;
        };
        let Ok(bytes) = game_assembly_function(rva) else {
            continue;
        };
        scan.runtime.initialize_references(rva, bytes)?;
        let no_return = Decoder::with_ip(64, bytes, (*GA_BASE + rva) as u64, DecoderOptions::NONE)
            .into_iter()
            .filter(|i| i.flow_control() == FlowControl::Call)
            .filter_map(|i| native::direct_target(&i))
            .filter(|&address| {
                *helper_cache.entry(address).or_insert_with(|| {
                    let Some(rva) = address.checked_sub(*GA_BASE) else {
                        return false;
                    };
                    let Ok(table) = game_assembly_functions() else {
                        return false;
                    };
                    let Some(range) = table.containing(rva) else {
                        return false;
                    };
                    range.start as usize == rva
                        && range.end - range.start <= 4096
                        && game_assembly_function(rva)
                            .is_ok_and(|b| response::trap_terminated(b, address))
                })
            })
            .collect();
        let analysis = response::analyze(
            bytes,
            *GA_BASE + rva,
            Register::R8,
            |slot| {
                scan.references
                    .class_from_slot(slot)
                    .filter(|c| response_classes.contains_key(c))
                    .map(|c| c.0)
            },
            |slot| strings.as_ref()?.read(slot),
            &loggers,
            &no_return,
        )
        .ok();
        let registered = by_rva
            .get(&rva)
            .filter(|types| types.len() == 1)
            .and_then(|types| types.iter().next().copied());
        let observed = analysis
            .as_ref()
            .filter(|a| a.classes.len() == 1)
            .and_then(|a| {
                response_classes
                    .get(&Il2CppClass(*a.classes.first().unwrap()))
                    .copied()
            });
        if registered.is_some_and(|ty| {
            analysis.as_ref().is_some_and(|a| {
                a.classes
                    .iter()
                    .any(|&c| c != ty.get_il2cpp_type().get_class().0)
            })
        }) {
            log::debug!("Response registry/cast disagreement at RVA 0x{rva:X}");
            continue;
        }
        let ty = registered.or(observed);
        if let Some(ty) = ty {
            let name = name
                .strip_suffix("ScRep")
                .map(|s| format!("{s}ScRsp"))
                .unwrap_or_else(|| name.to_owned());
            hints.context(signature.clone(), rva);
            hints.insert(ty.format_type_name(true), name.clone());
            if !name.ends_with("ScRsp") && !name.ends_with("ScNotify") {
                hints.entries.last_mut().unwrap().evidence = Evidence::Hint;
            }
            let diagnostics: Vec<_> = analysis
                .as_ref()
                .into_iter()
                .flat_map(|a| &a.diagnostics)
                .filter(|d| d.class == ty.get_il2cpp_type().get_class().0)
                .filter_map(|d| response_names::diagnostic_name(&d.literal, &name).map(|n| (n, d)))
                .collect();
            if !diagnostics.is_empty() {
                let callback = hints.entries.last_mut().unwrap();
                callback.evidence = Evidence::Hint;
                callback
                    .source
                    .push_str("; callback abbreviation superseded by failed-cast diagnostic");
                for (resolved, d) in diagnostics {
                    hints.insert(ty.format_type_name(true), resolved);
                    let c = hints.entries.last_mut().unwrap();
                    c.original_name = Some(name.clone());
                    c.source = format!(
                        "response argument CFG cast/null failure -> LogError; literal slot RVA 0x{:X}; call RVA 0x{:X}; {:?}",
                        d.slot - *GA_BASE,
                        d.call - *GA_BASE,
                        d.literal
                    );
                }
            }
        }
    }
    response_names::normalize(&mut hints);
    let result = hints.unique();
    scan.name_hints.extend(hints);
    log::info!(
        "Recovered {} response/notification names from verified handlers",
        result.len()
    );
    Ok(result)
}

struct ResponseStrings {
    class: Il2CppClass,
    length: usize,
    chars: usize,
}

impl ResponseStrings {
    fn new() -> Option<Self> {
        let class = il2cpp::get_cached_class("System.String")?;
        let ty = RuntimeType::from_class(class).ok()?;
        let fields = ty.get_fields_il2cpp();
        let offset = |name: &str, kind: &str| {
            let matches: Vec<_> = fields
                .iter()
                .filter(|f| {
                    f.is_instance()
                        && f.get_name().is_ok_and(|n| n.as_str() == name)
                        && f.get_field_type().is_ok_and(|t| t.il_name() == kind)
                })
                .collect();
            let [field] = matches.as_slice() else {
                return None;
            };
            usize::try_from(field.get_offset()).ok()
        };
        Some(Self {
            class,
            length: offset("m_stringLength", "System.Int32")?,
            chars: offset("m_firstChar", "System.Char")?,
        })
    }

    fn read(&self, slot: usize) -> Option<String> {
        use utils::memory::{read, read_bytes};
        let offset = slot.checked_sub(*GA_BASE)?;
        if offset.checked_add(8)? > il2cpp::profile::current().ok()?.assembly.image_size as usize {
            return None;
        }
        let pointer = unsafe { read::<usize>(slot) }.ok()?;
        if unsafe { read::<Il2CppClass>(pointer) }.ok()? != self.class {
            return None;
        }
        let length = unsafe { read::<i32>(pointer.checked_add(self.length)?) }.ok()?;
        if !(1..=256).contains(&length) {
            return None;
        }
        let bytes =
            unsafe { read_bytes(pointer.checked_add(self.chars)?, length as usize * 2) }.ok()?;
        let chars: Vec<_> = bytes
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        String::from_utf16(&chars).ok()
    }
}
