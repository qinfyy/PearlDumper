use std::{
    collections::{BTreeSet, HashMap, HashSet},
    time::{Duration, Instant},
};

use anyhow::Context;
use iced_x86::{
    Decoder, DecoderOptions, InstructionInfoFactory, Mnemonic, OpAccess, OpKind, Register,
};
use il2cpp::{FUNCTIONS_TABLE_REFLECTION, GA_BASE, vm::class::Il2CppClass};
use reflection::runtime_type::RuntimeType;
use utils::scanner::{
    game_assembly_code_sections, game_assembly_function, game_assembly_functions,
};

use super::{
    MessageMinimalInfo, native,
    recovery::{Evidence, Hints},
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
    let mut candidates: HashMap<String, BTreeSet<String>> = HashMap::new();
    for (signature, method) in methods {
        if !signature.ends_with("(System.UInt16,System.Object)") {
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
        let ty = match by_rva.get(&rva).filter(|types| types.len() == 1) {
            Some(types) => types.iter().next().copied(),
            None => {
                let Ok(bytes) = game_assembly_function(rva) else {
                    continue;
                };
                scan.runtime.initialize_references(rva, bytes)?;
                response_type(bytes, *GA_BASE + rva, &scan.references, &response_classes)
            }
        };
        if let Some(ty) = ty {
            let name = name
                .strip_suffix("ScRep")
                .map(|s| format!("{s}ScRsp"))
                .unwrap_or_else(|| name.to_owned());
            scan.name_hints.context(signature.clone(), rva);
            scan.name_hints
                .insert(ty.format_type_name(true), name.clone());
            if name.ends_with("ScRsp") || name.ends_with("ScNotify") {
                candidates
                    .entry(ty.format_type_name(true))
                    .or_default()
                    .insert(name);
            } else {
                scan.name_hints.entries.last_mut().unwrap().evidence = Evidence::Hint;
            }
        }
    }
    let mut result = HashMap::new();
    for (ty, names) in candidates {
        if names.len() == 1 {
            result.insert(ty, names.into_iter().next().unwrap());
        }
    }
    log::info!(
        "Recovered {} response/notification names from verified handlers",
        result.len()
    );
    Ok(result)
}

fn response_type(
    bytes: &[u8],
    ip: usize,
    references: &TypeReferences,
    responses: &HashMap<Il2CppClass, RuntimeType>,
) -> Option<RuntimeType> {
    let mut objects = HashSet::from([Register::R8]);
    let mut object_classes = HashSet::new();
    let mut known_classes = HashMap::new();
    let mut candidates = HashSet::new();
    let mut info = InstructionInfoFactory::new();
    for instruction in Decoder::with_ip(64, bytes, ip as u64, DecoderOptions::NONE) {
        if instruction.is_invalid() {
            return None;
        }
        if instruction.mnemonic() == Mnemonic::Cmp {
            let left = instruction.op0_register();
            let right = instruction.op1_register();
            let class = if object_classes.contains(&left) {
                if instruction.op1_kind() == OpKind::Memory
                    && instruction.is_ip_rel_memory_operand()
                {
                    references.class_from_slot(instruction.ip_rel_memory_address() as usize)
                } else {
                    known_classes.get(&right).copied()
                }
            } else if object_classes.contains(&right) {
                known_classes.get(&left).copied()
            } else {
                None
            };
            if let Some(ty) = class.and_then(|class| responses.get(&class)) {
                candidates.insert(*ty);
            }
        }

        let is_move =
            instruction.mnemonic() == Mnemonic::Mov && instruction.op0_kind() == OpKind::Register;
        let source = instruction.op1_register();
        let object_copy =
            is_move && instruction.op1_kind() == OpKind::Register && objects.contains(&source);
        let class_copy = is_move
            && instruction.op1_kind() == OpKind::Register
            && object_classes.contains(&source);
        let dereference = is_move
            && instruction.op1_kind() == OpKind::Memory
            && objects.contains(&instruction.memory_base())
            && instruction.memory_index() == Register::None
            && instruction.memory_displacement64() == 0
            && instruction.memory_size().size() == 8;
        let loaded_class = references.class_from_load(&instruction).or_else(|| {
            is_move
                .then(|| known_classes.get(&source).copied())
                .flatten()
        });

        for used in info.info(&instruction).used_registers() {
            if matches!(
                used.access(),
                OpAccess::Write
                    | OpAccess::CondWrite
                    | OpAccess::ReadWrite
                    | OpAccess::ReadCondWrite
            ) {
                let reg = used.register().full_register();
                objects.remove(&reg);
                object_classes.remove(&reg);
                known_classes.remove(&reg);
            }
        }
        if instruction.mnemonic() == Mnemonic::Call {
            for reg in [
                Register::RAX,
                Register::RCX,
                Register::RDX,
                Register::R8,
                Register::R9,
                Register::R10,
                Register::R11,
            ] {
                objects.remove(&reg);
                object_classes.remove(&reg);
                known_classes.remove(&reg);
            }
        }
        if is_move && instruction.op0_register().is_gpr64() {
            let destination = instruction.op0_register();
            if object_copy {
                objects.insert(destination);
            }
            if class_copy || dereference {
                object_classes.insert(destination);
            }
            if let Some(class) = loaded_class {
                known_classes.insert(destination, class);
            }
        }
    }
    (candidates.len() == 1).then(|| *candidates.iter().next().unwrap())
}
