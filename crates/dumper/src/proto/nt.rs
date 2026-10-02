use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use anyhow::Context;

use crate::proto::{
    FIGHT_GAME_SEND, IL2CPP_OBJECT_NEW_RVA, MessageMinimalInfo, NETWORK_MANAGER_SEND_NAME,
    NETWORK_MANAGER_SEND_VA, XLUA_OBJECT_TRANSLATOR_DELEGATE, XLUA_OBJECT_TRANSLATOR_METHOD_CLASS,
    XLUA_OBJECT_TRANSLATOR_STATIC_FIELDS_CLASS, XLUA_REGISTER_OBJECT_RVA,
};
use iced_x86::{Decoder, DecoderOptions, Instruction, Mnemonic, OpKind, Register};
use il2cpp::vm::method::Il2CppMethod;
use il2cpp::{
    FUNCTIONS_TABLE_REFLECTION, GA_BASE,
    api::Il2CppClass,
    get_cached_class, get_native_method,
    vm::{
        metadata_cache, native_collections::Dictionary, object::Il2CppObject, r#type::Il2CppType,
    },
};
use reflection::{field_info::FieldInfo, runtime_type::RuntimeType};
use std::borrow::Cow;
use utils::game_assembly_slice;

use super::type_references::TypeReferences;

pub fn get_rsp_notify_map() -> HashMap<RuntimeType, u16> {
    let mut process = false;

    let mut typedef_index = unsafe { il2cpp::ASSEMBLY_CSHARP_START };
    for _ in unsafe { typedef_index..il2cpp::MAX_TYPEDEFINDEX } {
        let class = metadata_cache::get_typeinfo_from_typedefindex(typedef_index);
        let runtime_type = RuntimeType::from_class(class).unwrap();
        if runtime_type.get_name().unwrap().as_str() == "NotifyType" {
            typedef_index += 2;
            process = true;
            continue;
        }

        if process {
            if let Some(dictionary) = runtime_type
                .get_fields(62)
                .iter()
                .find(|v| {
                    v.get_field_type().unwrap().format_type_name(true)
                        == "Dictionary<RuntimeTypeHandle, ushort>"
                })
                .map(|v| v.get_value(Il2CppObject::NULL).unwrap())
            {
                let dict = unsafe { *(dictionary.0 as *const Dictionary<Il2CppType, u16>) };

                return dict
                    .iter()
                    .map(|(ty, cmdid)| (RuntimeType::from_il2cpp_type(ty).unwrap(), cmdid))
                    .collect();
            }
            log::debug!("[Proto Dumper] cannot find nt field!");

            break;
        }

        typedef_index += 1;
        continue;
    }

    HashMap::new()
}

fn get_req_method_va_name_map(
    references: &TypeReferences,
) -> anyhow::Result<HashMap<usize, String>> {
    let mut output = HashMap::new();
    let methods = FUNCTIONS_TABLE_REFLECTION
        .get()
        .context("IL2CPP reflection method cache is not initialized")?;
    log::info!(
        "Indexing {} cached methods for XLua name recovery",
        methods.len()
    );
    let started = Instant::now();
    let mut last_progress = started;
    let mut known_methods = HashMap::<usize, Cow<'static, str>>::new();
    let mut unique_methods = HashMap::<Cow<'static, str>, Vec<Il2CppMethod>>::new();
    for (index, method) in methods.values().enumerate() {
        if index % 256 == 0 && last_progress.elapsed() >= Duration::from_secs(2) {
            log::info!(
                "XLua method index: {index}/{} methods, {:.1}s elapsed",
                methods.len(),
                started.elapsed().as_secs_f64()
            );
            last_progress = Instant::now();
        }
        let name = method.get_name();
        known_methods.insert(method.0, name.clone());
        unique_methods.entry(name).or_default().push(*method);
    }
    log::info!(
        "XLua method index ready in {:.2}s",
        started.elapsed().as_secs_f64()
    );
    let mappings = disasm_obf_deobf_method_by_xlua_obj_translator(references, &known_methods)?;

    for (obf, deobf) in mappings {
        if !deobf.ends_with("Req") {
            continue;
        }

        let Some(methods) = unique_methods.get(&Cow::Borrowed(obf.as_str())) else {
            continue;
        };

        for method in methods {
            if method.class().byval_arg().il_name() == *XLUA_OBJECT_TRANSLATOR_METHOD_CLASS {
                continue;
            }

            output.insert(method.va(), deobf.clone());
        }
    }

    Ok(output)
}

fn disasm_obf_deobf_method_by_xlua_obj_translator(
    references: &TypeReferences,
    known_methods: &HashMap<usize, Cow<'static, str>>,
) -> anyhow::Result<HashMap<String, String>> {
    let mut output = HashMap::new();

    let delegate_class = get_cached_class(&XLUA_OBJECT_TRANSLATOR_DELEGATE).ok_or_else(|| {
        anyhow::anyhow!(
            "missing XLua delegate class {}",
            *XLUA_OBJECT_TRANSLATOR_DELEGATE
        )
    })?;

    let obj_translator_fields_class = get_cached_class(&XLUA_OBJECT_TRANSLATOR_STATIC_FIELDS_CLASS)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "missing XLua static field class {}",
                *XLUA_OBJECT_TRANSLATOR_STATIC_FIELDS_CLASS
            )
        })?;
    let obj_translator_fields = obj_translator_fields_class
        .get_fields()
        .into_iter()
        .map(|v| {
            let name = FieldInfo::from_il2cpp_field(v)?.get_name()?.as_str();
            let (_, suffix) = name
                .split_once("__")
                .ok_or_else(|| anyhow::anyhow!("unsupported XLua delegate field name {name}"))?;
            Ok((v.offset(), strip_prefixes(suffix, &["Send"]).to_string()))
        })
        .collect::<anyhow::Result<HashMap<_, _>>>()?;

    let slice = game_assembly_slice();
    let xlua_register_object_rva = *XLUA_REGISTER_OBJECT_RVA;
    let mut decoder = Decoder::with_ip(
        64,
        slice
            .get(xlua_register_object_rva..)
            .context("XLua registration RVA is outside GameAssembly")?,
        *GA_BASE as u64 + xlua_register_object_rva as u64,
        DecoderOptions::NONE,
    );

    let mut instructions = VecDeque::<Instruction>::with_capacity(500);
    let mut instruction = Instruction::default();

    let mut static_field_offset = None;
    let mut push_cnt = 0;
    let mut past_prologue = false;
    let mut delegate_loads = 0;
    let mut null_method_refs = 0;
    let mut unknown_method_refs = 0;
    log::info!("Scanning XLua registration method at RVA 0x{xlua_register_object_rva:X}");

    while decoder.can_decode() {
        decoder.decode_out(&mut instruction);

        if instruction.mnemonic() == Mnemonic::Push {
            if past_prologue {
                push_cnt += 1;

                if push_cnt > 4 {
                    break;
                }
            }
        } else {
            past_prologue = true;
        }

        // mov rcx, cs:XLUA_DELEGATE_TYPE_INFO_VA
        if instruction.op0_register() == Register::RCX
            && references.class_from_load(&instruction) == Some(delegate_class)
        {
            delegate_loads += 1;
            // traverse to find the displacement register
            let mut found_offset = None;
            for i in (0..instructions.len()).rev() {
                let inst = instructions[i];

                if inst.mnemonic() != Mnemonic::Mov {
                    continue;
                }

                let offset = (is_gp64_register(inst.op0_register())
                    && inst.op1_kind() == OpKind::Memory
                    || inst.op0_kind() == OpKind::Memory)
                    .then(|| inst.memory_displacement64());

                let Some(offset) = offset else {
                    continue;
                };

                if obj_translator_fields.contains_key(&(offset as usize)) {
                    found_offset = Some(offset);
                    break;
                }
            }

            if let Some(offset) = found_offset {
                static_field_offset = Some(offset);
            }

            continue;
        }

        // static_field_offset already set
        // call to il2cpp_object_new
        let il2cpp_object_new_rva = *IL2CPP_OBJECT_NEW_RVA;
        if let Some(offset) = static_field_offset
            && instruction.mnemonic() == Mnemonic::Call
            && instruction.near_branch_target() == (*il2cpp::GA_BASE + il2cpp_object_new_rva) as u64
        {
            decoder.decode_out(&mut instruction); // skip mov rsi, rax
            decoder.decode_out(&mut instruction);

            // mov rax, cs::METHOD_INFO_VA
            if let Some(type_va) = references.slot_address(&instruction) {
                let method = unsafe { utils::memory::read::<usize>(type_va) }
                    .with_context(|| format!("reading XLua method reference at 0x{type_va:X}"))?;
                if method == 0 {
                    null_method_refs += 1;
                    static_field_offset = None;
                    continue;
                }
                let Some(name) = known_methods.get(&method) else {
                    unknown_method_refs += 1;
                    log::debug!(
                        "Skipping unknown XLua method reference 0x{method:X} at 0x{type_va:X}"
                    );
                    static_field_offset = None;
                    continue;
                };

                let Some(field_name) = obj_translator_fields.get(&(offset as usize)) else {
                    static_field_offset = None;
                    continue;
                };

                output.insert(name.to_string(), field_name.to_string());
                static_field_offset = None;
            }

            continue;
        }

        if instructions.len() == 500 {
            instructions.pop_front();
        }
        instructions.push_back(instruction);
    }

    log::info!(
        "XLua reference scan: {delegate_loads} delegate loads, {} names, {null_method_refs} null method references, {unknown_method_refs} unknown method references",
        output.len()
    );
    anyhow::ensure!(
        !output.is_empty(),
        "no XLua request-name mappings recovered at RVA 0x{xlua_register_object_rva:X} ({delegate_loads} delegate loads, {null_method_refs} null and {unknown_method_refs} unknown method references); metadata usages may not be initialized or the registration layout is unsupported"
    );
    log::info!("Recovered {} XLua delegate names", output.len());
    Ok(output)
}

pub(super) struct RequestScan {
    references: TypeReferences,
    targets: HashMap<usize, ReqFlavor>,
    method_names: HashMap<usize, String>,
}

pub(super) fn prepare_request_scan() -> anyhow::Result<RequestScan> {
    log::info!(
        "Preparing request scan from live IL2CPP class references (no script TYPE_INFOS required)"
    );
    let references = TypeReferences::from_runtime()?;
    let signature = format!(
        "RPG.Client.NetworkManager::{}(System.UInt16,Google.Protobuf.IMessage,System.Boolean)",
        *NETWORK_MANAGER_SEND_NAME
    );
    let send = get_native_method(&signature)
        .ok_or_else(|| anyhow::anyhow!("missing request send method {signature}"))?
        .checked_va()?;
    let targets = HashMap::from([
        (send, ReqFlavor::Standard),
        (*NETWORK_MANAGER_SEND_VA, ReqFlavor::Standard),
        (*FIGHT_GAME_SEND, ReqFlavor::Fight),
    ]);
    for (&address, flavor) in &targets {
        anyhow::ensure!(
            utils::memory::is_executable(address),
            "invalid {flavor:?} request send address 0x{address:X}"
        );
        log::info!("Request send target {flavor:?}: 0x{address:X}");
    }
    log::info!("Recovering XLua request method names");
    let method_names = get_req_method_va_name_map(&references)?;
    log::info!(
        "Request scan ready: {} send targets, {} named methods",
        targets.len(),
        method_names.len()
    );
    Ok(RequestScan {
        references,
        targets,
        method_names,
    })
}

pub(super) fn get_req_map(
    scan: &RequestScan,
    minimal_info: &HashMap<RuntimeType, MessageMinimalInfo>,
    rsp_notify_map: &HashMap<RuntimeType, u16>,
    req_map: &mut HashMap<RuntimeType, (u16, Option<String>)>,
) -> anyhow::Result<HashMap<RuntimeType, Vec<String>>> {
    let mut request_classes = HashMap::new();
    for ty in minimal_info.keys() {
        if !rsp_notify_map.contains_key(ty) && !ty.get_isenum()?.unbox() {
            request_classes.insert(ty.get_il2cpp_type().get_class(), *ty);
        }
    }
    anyhow::ensure!(
        !request_classes.is_empty(),
        "no request message classes available for packet ID recovery"
    );
    log::info!(
        "Scanning native send calls for {} request classes",
        request_classes.len()
    );
    let rvas = disasm_all_req(scan, &request_classes, rsp_notify_map, req_map);
    anyhow::ensure!(
        !req_map.is_empty(),
        "no request packet IDs recovered from live type references; metadata usages may not be initialized or the send-call layout is unsupported"
    );
    Ok(rvas)
}

#[derive(Debug, Eq, PartialEq, Clone, Copy)]
enum ReqFlavor {
    Standard, // DX, R8
    Fight,    // R8, R9
}

fn disasm_all_req(
    scan: &RequestScan,
    request_classes: &HashMap<Il2CppClass, RuntimeType>,
    rsp_notify_map: &HashMap<RuntimeType, u16>,
    out: &mut HashMap<RuntimeType, (u16, Option<String>)>,
) -> HashMap<RuntimeType, Vec<String>> {
    let va_deobf_map = &scan.method_names;
    let started = Instant::now();
    let mut last_progress = started;
    let mut instruction = Instruction::default();
    let mut instructions = VecDeque::<Instruction>::with_capacity(500);
    let mut req_rvas: HashMap<RuntimeType, Vec<String>> = HashMap::new();

    #[derive(Debug, Eq, PartialEq, Clone, Copy)]
    enum InstType {
        Memory { base: Register, displacement: i64 },
        Normal(Register),
    }

    impl InstType {
        pub fn new(op_kind: OpKind, reg: Register, memory: Register, displacement: i64) -> Self {
            if op_kind == OpKind::Memory {
                Self::Memory {
                    base: memory,
                    displacement,
                }
            } else {
                Self::Normal(reg)
            }
        }
        pub fn is_rax(&self) -> bool {
            match self {
                InstType::Memory { base, .. } => *base == Register::RAX,
                InstType::Normal(register) => *register == Register::RAX,
            }
        }
        pub fn is_none(&self) -> bool {
            match self {
                InstType::Memory { base, .. } => *base == Register::None,
                InstType::Normal(register) => *register == Register::None,
            }
        }
    }

    type CandidateInfo = (HashSet<u16>, HashSet<Option<String>>, Vec<String>);
    let mut candidates: HashMap<RuntimeType, CandidateInfo> = HashMap::new();
    for (rva, bytes) in utils::scanner::game_assembly_code_sections() {
        log::info!(
            "Scanning executable region at RVA 0x{rva:X}: {} bytes",
            bytes.len()
        );
        let mut decoder =
            Decoder::with_ip(64, bytes, (*GA_BASE + rva) as u64, DecoderOptions::NONE);
        let mut cur_func_va = None;
        let mut next_progress_position = 65_536;
        instructions.clear();
        while decoder.can_decode() {
            if decoder.position() >= next_progress_position {
                if last_progress.elapsed() >= Duration::from_secs(2) {
                    log::info!(
                        "Request scan: region RVA 0x{rva:X}, {}/{} bytes, {} candidate types, {:.1}s elapsed",
                        decoder.position(),
                        bytes.len(),
                        candidates.len(),
                        started.elapsed().as_secs_f64()
                    );
                    last_progress = Instant::now();
                }
                next_progress_position = decoder.position().saturating_add(65_536);
            }
            decoder.decode_out(&mut instruction);

            if instruction.mnemonic() == Mnemonic::Push
                && let Some(prev) = instructions.back()
                && prev.mnemonic() != Mnemonic::Push
            {
                cur_func_va = Some(instruction.ip());
                instructions.clear();
            }

            if (instruction.mnemonic() == Mnemonic::Jmp || instruction.mnemonic() == Mnemonic::Call)
                && let Some(&flavor) = scan
                    .targets
                    .get(&(instruction.near_branch_target() as usize))
            {
                let mut obj_register = None;
                let mut cmd_id = None;
                let mut push_rva = None;
                let mut last_push_index = None;

                let mut i = instructions.len();
                while i > 0 {
                    i -= 1;
                    if instructions[i].mnemonic() == Mnemonic::Push {
                        last_push_index = Some(i);
                    } else if last_push_index.is_some() {
                        break;
                    }
                }
                if let Some(push_index) = last_push_index {
                    push_rva = Some(instructions[push_index].ip() as usize - *GA_BASE);
                }

                for i in (0..instructions.len()).rev() {
                    let inst = instructions[i];
                    if inst.mnemonic() == Mnemonic::Push {
                        break;
                    }

                    // 1: CmdId
                    if cmd_id.is_none() && inst.mnemonic() == Mnemonic::Mov {
                        let reg = inst.op0_register();
                        let is_match = match flavor {
                            ReqFlavor::Standard => reg == Register::DX,
                            ReqFlavor::Fight => reg == Register::R8 || reg == Register::R8W,
                        };
                        if is_match {
                            let id = inst.immediate16();
                            if id != 0 && !rsp_notify_map.values().any(|&v| v == id) {
                                cmd_id = Some(id);
                            }
                        }
                        if cmd_id.is_some() {
                            continue;
                        }
                    }

                    // 2: Object Register
                    if obj_register.is_none() && inst.mnemonic() == Mnemonic::Mov {
                        let reg = inst.op0_register();
                        let is_match = match flavor {
                            ReqFlavor::Standard => reg == Register::R8,
                            ReqFlavor::Fight => reg == Register::R9,
                        };
                        if is_match {
                            obj_register = Some(InstType::new(
                                inst.op1_kind(),
                                inst.op1_register(),
                                inst.memory_base(),
                                inst.memory_displacement64() as i64,
                            ));
                            continue;
                        }
                    }

                    // 3: register
                    if let Some(reg) = obj_register
                        && inst.mnemonic() == Mnemonic::Mov
                        && (InstType::new(
                            inst.op0_kind(),
                            inst.op0_register(),
                            inst.memory_base(),
                            inst.memory_displacement64() as i64,
                        ) == reg)
                        && !reg.is_rax()
                    {
                        let new = InstType::new(
                            inst.op1_kind(),
                            inst.op1_register(),
                            inst.memory_base(),
                            inst.memory_displacement64() as i64,
                        );
                        if !new.is_none() {
                            obj_register = Some(new);
                        }
                        continue;
                    }

                    // 4: Identification
                    if let Some(cmd_id) = cmd_id
                        && let Some(reg) = obj_register
                    {
                        // A: il2cpp_object_new
                        if reg.is_rax()
                            && (inst.mnemonic() == Mnemonic::Call
                                || inst.mnemonic() == Mnemonic::Jmp)
                        {
                            let target_va = inst.near_branch_target() as usize;
                            if target_va.checked_sub(*GA_BASE) == Some(*IL2CPP_OBJECT_NEW_RVA)
                                && let Some(previous) =
                                    i.checked_sub(1).map(|index| &instructions[index])
                                && let Some(class) = scan.references.class_from_load(previous)
                                && let Some(&rt) = request_classes.get(&class)
                            {
                                let deobf_name = cur_func_va
                                    .and_then(|v| va_deobf_map.get(&(v as usize)).cloned());
                                let entry = candidates.entry(rt).or_insert_with(|| {
                                    (HashSet::new(), HashSet::new(), Vec::new())
                                });
                                entry.0.insert(cmd_id);
                                entry.1.insert(deobf_name);
                                if let Some(prva) = push_rva {
                                    entry.2.push(format!("0x{prva:X}"));
                                }
                                break;
                            }
                        }

                        // B: Cmp
                        if flavor == ReqFlavor::Standard
                            && inst.mnemonic() == Mnemonic::Cmp
                            && inst.op0_kind() == OpKind::Memory
                            && inst.memory_base()
                                == match reg {
                                    InstType::Normal(r) => r,
                                    InstType::Memory { base, .. } => base,
                                }
                        {
                            let type_info_reg = inst.op1_register();
                            if type_info_reg != Register::None {
                                for j in (0..i).rev() {
                                    let prev = instructions[j];
                                    if prev.mnemonic() == Mnemonic::Mov
                                        && prev.op0_register() == type_info_reg
                                    {
                                        if let Some(class) = scan.references.class_from_load(&prev)
                                            && let Some(&rt) = request_classes.get(&class)
                                        {
                                            let deobf_name = cur_func_va.and_then(|v| {
                                                va_deobf_map.get(&(v as usize)).cloned()
                                            });
                                            let entry = candidates.entry(rt).or_insert_with(|| {
                                                (HashSet::new(), HashSet::new(), Vec::new())
                                            });
                                            entry.0.insert(cmd_id);
                                            entry.1.insert(deobf_name);
                                            if let Some(prva) = push_rva {
                                                entry.2.push(format!("0x{prva:X}"));
                                            }
                                            break;
                                        }
                                    }
                                }
                                if candidates.values().any(|v| v.0.contains(&cmd_id)) {
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            if instructions.len() >= 500 {
                instructions.pop_front();
            }
            instructions.push_back(instruction);
        }
    }

    let candidate_count = candidates.len();
    for (rt, (ids, names, rvas)) in candidates {
        if ids.len() == 1 {
            out.insert(
                rt,
                (
                    *ids.iter().next().unwrap(),
                    names.into_iter().flatten().next(),
                ),
            );
            req_rvas.insert(rt, rvas);
        }
    }
    log::info!(
        "Request scan complete: {} unique packet IDs from {candidate_count} candidate types in {:.2}s",
        out.len(),
        started.elapsed().as_secs_f64()
    );
    req_rvas
}

pub fn get_rsp_notify_names() -> HashMap<String, String> {
    let cached_methods = FUNCTIONS_TABLE_REFLECTION.get().unwrap();
    let prefixes_to_replace = ["_OnCmd", "_Cmd", "_On", "OnCmd", "On", "Cmd"];

    let mut nt_map = HashMap::new();

    for (m_name, m) in cached_methods {
        if !m_name.ends_with("(System.UInt16,System.Object)") {
            continue;
        }

        let Some(proto_class) = disasm_rsp_notify_3_args(m.rva()) else {
            continue;
        };

        let m_name = m.get_name();
        for prefix in prefixes_to_replace {
            if let Some(name) = m_name.strip_prefix(prefix) {
                let _ = microseh::try_seh(|| {
                    nt_map.insert(
                        RuntimeType::from_class(proto_class)
                            .unwrap()
                            .format_type_name(true),
                        name.to_string()
                            .replace("Cmd", "")
                            .replace("ScRep", "ScRsp"),
                    );
                });
            }
        }
    }

    nt_map
}

pub fn get_rsp_notify_method_rvas() -> HashMap<String, Vec<String>> {
    let cached_methods = FUNCTIONS_TABLE_REFLECTION.get().unwrap();
    let prefixes_to_replace = ["_OnCmd", "_Cmd", "_On", "OnCmd", "On", "Cmd"];

    let mut rva_map: HashMap<String, Vec<String>> = HashMap::new();

    for (m_name, m) in cached_methods {
        if !m_name.ends_with("(System.UInt16,System.Object)") {
            continue;
        }

        let Some(proto_class) = disasm_rsp_notify_3_args(m.rva()) else {
            continue;
        };

        let m_name_str = m.get_name();
        for prefix in prefixes_to_replace {
            if m_name_str.starts_with(prefix) {
                let _ = microseh::try_seh(|| {
                    let formatted_name = RuntimeType::from_class(proto_class)
                        .unwrap()
                        .format_type_name(true)
                        .replace("ScRep", "ScRsp");

                    rva_map
                        .entry(formatted_name)
                        .or_default()
                        .push(format!("0x{:X}", m.rva()));
                });
                break;
            }
        }
    }

    rva_map
}

fn disasm_rsp_notify_3_args(rva: usize) -> Option<Il2CppClass> {
    let slice = game_assembly_slice();
    let mut decoder = Decoder::with_ip(
        64,
        &slice[rva..],
        (*GA_BASE + rva) as u64,
        DecoderOptions::NONE,
    );

    let mut instruction = Instruction::default();

    let mut is_passing_push = false;
    let mut current_r8_reg = Register::R8;
    let mut current_dereferenced_reg = None;

    while decoder.can_decode() {
        decoder.decode_out(&mut instruction);

        // Move current r8 reg into another reg
        // mov new_current_r8_reg, current_r8_reg
        if instruction.mnemonic() == Mnemonic::Mov && instruction.op1_register() == current_r8_reg {
            current_r8_reg = instruction.op0_register();
            is_passing_push = true;
        }

        // detect if current_r8_reg is being dereferenced
        // mov current_dereferenced_reg, [current_r8_reg]
        if instruction.op1_kind() == OpKind::Memory && instruction.memory_base() == current_r8_reg {
            current_dereferenced_reg = Some(instruction.op0_register());
        }

        // cmp current_dereferenced_reg, cs:PROTO_TYPE
        if instruction.mnemonic() == Mnemonic::Cmp
            && let Some(reg) = current_dereferenced_reg
            && instruction.op0_register() == reg
            && instruction.op1_kind() == OpKind::Memory
        {
            let va = instruction.memory_displacement64() as usize;
            match microseh::try_seh(|| {
                let class = unsafe { *(va as *const Il2CppClass) };
                if class.0 != 0 { Some(class) } else { None }
            }) {
                Ok(data) => return data,
                Err(_err) => {
                    return None;
                }
            }
        }

        // already out of current sub_
        if is_passing_push && instruction.mnemonic() == Mnemonic::Push {
            break;
        }
    }

    None
}

fn strip_prefixes<'a>(s: &'a str, prefixes: &[&str]) -> &'a str {
    for p in prefixes {
        if let Some(rest) = s.strip_prefix(p) {
            return rest;
        }
    }
    s
}

fn is_gp64_register(register: Register) -> bool {
    matches!(
        register,
        Register::RAX
            | Register::RCX
            | Register::RDX
            | Register::RBX
            | Register::RSP
            | Register::RBP
            | Register::RSI
            | Register::RDI
            | Register::R8
            | Register::R9
            | Register::R10
            | Register::R11
            | Register::R12
            | Register::R13
            | Register::R14
            | Register::R15
    )
}
