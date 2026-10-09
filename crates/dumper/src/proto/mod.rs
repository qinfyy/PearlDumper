use std::{
    collections::HashMap,
    fs::File,
    io::{BufWriter, Write},
    sync::OnceLock,
    time::{Duration, Instant},
};

use anyhow::Context;
use cache::{CachedType, TypeCache};
use il2cpp::{get_cached_class, get_native_method, vm::metadata_cache};
use reflection::{property_info::PropertyInfo, runtime_type::RuntimeType};

mod cache;
mod discovery;
mod fields;
pub mod handler_nt;
mod logic_nt;
mod merge_from;
mod method_nt;
mod native;
mod nt;
mod output;
mod proto_asm_parser;
mod proto_stream;
pub(crate) mod recovery;
mod runtime;
mod type_references;
pub mod util;
mod write_to;

static IL2CPP_OBJECT_NEW_RVA: OnceLock<usize> = OnceLock::new();

// The 4.6.51 response base BHCEEHPIGFL retains the Retcode property name.
const RETCODE_FIELD_NAME: &str = "Retcode";

const CODED_INPUT_STREAM: &str = "Google.Protobuf.CodedInputStream";
const MERGE_FROM: &str = "MergeFrom";
const CODED_OUTPUT_STREAM: &str = "Google.Protobuf.CodedOutputStream";
const WRITE_TO: &str = "WriteTo";
const UNKNOWN_FIELD_SET: &str = "Google.Protobuf.UnknownFieldSet";
const BYTE_STRING: &str = "Google.Protobuf.ByteString";
const PROTOBUF_ANY: &str = "MiHoYo.SDK.Protobuf.WellKnownTypes.Any";
const GET_COUNT_PROPERTY: &str = "Count";

pub struct MessageMinimalInfo {
    #[allow(unused)]
    pub cmd_id: u16,
    pub fields: Vec<FieldMinimalInfo>,
    pub write_to_rva: usize,
    pub merge_from_rva: usize,
}

impl MessageMinimalInfo {
    pub fn new(cmd_id: u16) -> Self {
        Self {
            cmd_id,
            fields: Vec::new(),
            write_to_rva: 0,
            merge_from_rva: 0,
        }
    }
}

pub struct FieldMinimalInfo {
    pub tag: u32,
    #[allow(unused)]
    pub xor: u32,
    pub offset: u32,
    pub oneof_extra_data: Option<OneofVariantInfo>,
    pub number_type: NumberType,
    pub property: Option<PropertyInfo>,
}

#[derive(Clone, Copy)]
pub enum NumberType {
    None,
    Varint,
    Normal,
    #[allow(unused)]
    ZigZagVarint,
}

pub struct OneofVariantInfo {
    pub oneof_enum_offset: u32,
    pub variant_type: RuntimeType,
    pub property: Option<PropertyInfo>,
}

#[allow(dead_code)]
pub enum ProtoDumpMode {
    NativeGetters,
    ClassFieldNumber,
    MergeFrom,
    WriteTo,
    Asm,
}

pub fn dump<W: Write>(
    out: &mut W,
    cmdid_out: &mut W,
    dump_mode: ProtoDumpMode,
    enable_logging: bool,
) -> anyhow::Result<()> {
    recovery::begin()?;
    let result = crate::guarded(|| dump_inner(out, cmdid_out, dump_mode, enable_logging));
    if let Err(error) = &result {
        if let Err(report_error) = recovery::record_failure(error) {
            log::error!("Cannot record NT failure: {report_error:#}");
        }
    }
    result
}

fn dump_inner<W: Write>(
    out: &mut W,
    cmdid_out: &mut W,
    dump_mode: ProtoDumpMode,
    enable_logging: bool,
) -> anyhow::Result<()> {
    let started = Instant::now();
    let mut request_scan = nt::prepare_request_scan()?;
    let field_decoder = fields::FieldDecoder::new()?;
    let type_cache = TypeCache::init();
    if matches!(dump_mode, ProtoDumpMode::MergeFrom | ProtoDumpMode::WriteTo) {
        proto_stream::init();
    }
    let rsp_notify_map = request_scan.registry.responses.clone();
    let mut req_map = HashMap::new();
    let mut minimal_info_map = HashMap::<RuntimeType, MessageMinimalInfo>::new();
    log::info!("Recovering protobuf message fields");
    let mut last_progress = Instant::now();
    for i in unsafe { il2cpp::RPG_NETWORK_PROTO_START..il2cpp::RPG_NETWORK_PROTO_END } {
        if last_progress.elapsed() >= Duration::from_secs(2) {
            log::info!(
                "Protocol fields: type index {i}/{}, {} messages, {:.1}s elapsed",
                unsafe { il2cpp::RPG_NETWORK_PROTO_END },
                minimal_info_map.len(),
                started.elapsed().as_secs_f64()
            );
            last_progress = Instant::now();
        }
        let proto_class = metadata_cache::get_typeinfo_from_typedefindex(i);
        let proto_type = RuntimeType::from_class(proto_class)
            .with_context(|| format!("resolving protocol type index {i}"))?;
        if proto_type.find_method_il2cpp(MERGE_FROM).is_none() {
            continue;
        }
        let proto_name = proto_type.il_name();
        if enable_logging {
            log::debug!("[Proto Dumper] Generating fields for {proto_name}");
        }
        let mut message_info = MessageMinimalInfo::new(
            request_scan
                .registry
                .all
                .get(&proto_type)
                .copied()
                .unwrap_or(0),
        );
        let write_to_method =
            get_native_method(&format!("{proto_name}::{WRITE_TO}({CODED_OUTPUT_STREAM})"))
                .with_context(|| {
                    format!("missing {proto_name}::{WRITE_TO}({CODED_OUTPUT_STREAM})")
                })?;
        message_info.merge_from_rva =
            get_native_method(&format!("{proto_name}::{MERGE_FROM}({CODED_INPUT_STREAM})"))
                .with_context(|| {
                    format!("missing {proto_name}::{MERGE_FROM}({CODED_INPUT_STREAM})")
                })?
                .checked_rva()?;
        message_info.write_to_rva = write_to_method.checked_rva()?;
        match dump_mode {
            ProtoDumpMode::NativeGetters => {
                field_decoder
                    .recover(proto_type, &mut message_info)
                    .with_context(|| format!("recovering fields of {proto_name}"))?;
            }
            ProtoDumpMode::ClassFieldNumber => {
                util::generate_minimal_info_from_constants(
                    proto_type,
                    &mut message_info,
                    &type_cache,
                );
            }
            ProtoDumpMode::Asm => {
                proto_asm_parser::dump_from_write_to_asm(&proto_name, &mut message_info);
            }
            ProtoDumpMode::MergeFrom => {
                let proto_instance = proto_class.create_instance();
                proto_type
                    .find_method_il2cpp(".ctor")
                    .context("missing protobuf constructor")?
                    .get_il2cpp_method()
                    .invoke::<usize>(proto_instance, &[])?;
                merge_from::dump_merge_from(
                    proto_type,
                    proto_instance,
                    &mut message_info,
                    &type_cache,
                );
            }
            ProtoDumpMode::WriteTo => {
                let ctor_method = get_native_method(&format!("{proto_name}::.ctor()"))
                    .with_context(|| format!("missing {proto_name}::.ctor()"))?;
                write_to::dump_writeto(
                    enable_logging,
                    i,
                    proto_type,
                    ctor_method,
                    write_to_method,
                    &mut message_info,
                );
            }
        }
        minimal_info_map.insert(proto_type, message_info);
    }

    let req_rvas = nt::get_req_map(&mut request_scan, &minimal_info_map, &mut req_map)?;
    let rsp_notify_names = nt::get_rsp_notify_names(&mut request_scan)?;

    // Registry coverage and wire fields are independent of optional name hints.
    // Publish a separately named checkpoint before any naming handler can fail.
    log::info!("Writing core protocol checkpoint before name recovery");
    let (core_names, mut type_to_item) = {
        let mut core_proto = BufWriter::new(File::create("DUMP/StarRail.core.proto.partial")?);
        let mut core_ids = BufWriter::new(File::create("DUMP/packetIds.core.json.partial")?);
        let (cmd_ids, names, items) = output::generate_protobuf(
            &type_cache,
            &minimal_info_map,
            &rsp_notify_map,
            &req_map,
            &rsp_notify_names,
            &HashMap::new(),
            &HashMap::new(),
            &mut core_proto,
        )
        .context("generating core protocol checkpoint")?;
        validate_packet_ids(&cmd_ids, &items, &request_scan.registry.all)?;
        serde_json::to_writer_pretty(&mut core_ids, &cmd_ids)?;
        writeln!(core_ids)?;
        core_proto.flush()?;
        core_ids.flush()?;
        core_proto.get_ref().sync_all()?;
        core_ids.get_ref().sync_all()?;
        drop((core_proto, core_ids));
        std::fs::rename(
            "DUMP/StarRail.core.proto.partial",
            "DUMP/StarRail.core.proto",
        )?;
        std::fs::rename(
            "DUMP/packetIds.core.json.partial",
            "DUMP/packetIds.core.json",
        )?;
        log::info!(
            "Core protocol checkpoint verified: {} messages, {} packet IDs; full name recovery is still pending",
            minimal_info_map.len(),
            cmd_ids.len()
        );
        (names, items)
    };

    let mut recovery = recovery::Recovery::new(&type_to_item, &core_names)?;
    recovery.add(std::mem::take(&mut request_scan.name_hints));
    log::info!("Recovering protocol names and field names from handlers");
    let (method_handler_map, method_nt_map) = method_nt::get_method_nt_map()
        .context("protocol name recovery failed; verified core protocol files are retained")?;
    recovery.add(method_nt_map);
    let (initial_names, _) = recovery.resolve();
    // Only the central decision set may rename the final model.
    for (_, name) in req_map.values_mut() {
        *name = None;
    }
    let rsp_notify_names = HashMap::new();
    log::info!("Building named protocol model for field recovery");
    let (_, proto_name_map) = output::finish_protobuf(
        &mut type_to_item,
        &core_names,
        &rsp_notify_map,
        &req_map,
        &rsp_notify_names,
        &initial_names,
        &HashMap::new(),
        std::io::sink(),
    )?;

    let mut cs_type_infos: HashMap<String, Vec<String>> = HashMap::new();
    let mut table_entries = Vec::new();
    for (ty, rvas) in &req_rvas {
        let formatted = ty.format_type_name(true);
        let name = proto_name_map.get(&formatted).cloned().unwrap_or(formatted);
        cs_type_infos.insert(
            name.clone(),
            rvas.iter().map(|rva| format!("0x{rva:X}")).collect(),
        );
        for &rva in rvas {
            table_entries.push((ty.il_name().into_owned(), name.clone(), rva));
        }
    }
    let _ = handler_nt::CS_HANDLER_TABLE.set(table_entries);

    // Registered delegates are the authoritative receive handlers. Iterating every
    // method parameter also included serializers and constructors in this output.
    let mut sc_packet_handlers: HashMap<String, Vec<String>> = HashMap::new();
    for (ty, rvas) in &request_scan.registry.handlers {
        let formatted = ty.format_type_name(true);
        let name = proto_name_map.get(&formatted).cloned().unwrap_or(formatted);
        sc_packet_handlers
            .entry(name)
            .or_default()
            .extend(rvas.iter().map(|rva| format!("0x{rva:X}")));
    }
    for (name, rvas) in method_handler_map {
        let name = proto_name_map.get(&name).cloned().unwrap_or(name);
        sc_packet_handlers.entry(name).or_default().extend(rvas);
    }
    for rvas in sc_packet_handlers.values_mut() {
        rvas.retain(|rva| rva != "0x0");
        rvas.sort_unstable();
        rvas.dedup();
    }

    log::info!("Recovering global protocol field names");
    let mut global_hints = crate::guarded(|| Ok(method_nt::dump_global_field_map()))
        .context("global protocol field names")?;
    for c in &mut global_hints.entries {
        c.symbol = format!("legacy-field:{}", c.symbol);
        c.evidence = recovery::Evidence::Hint;
    }
    recovery.add(global_hints);
    log::info!("Recovering protocol field names from packet handlers");
    let handler_fields =
        handler_nt::get_handler_nt_map(&type_to_item).context("packet handler field names")?;
    recovery.add(handler_fields);
    let (type_names, field_names) = recovery.resolve();
    log::info!("Applying protocol logic name rules");
    let recovered = crate::guarded(|| {
        Ok(logic_nt::run_logic_nt(
            &type_to_item.values().cloned().collect::<Vec<_>>(),
            &type_names,
            &field_names,
        ))
    })
    .context("protocol logic name rules")?;
    recovery.add(recovered);
    let (type_names, field_names) = recovery.resolve();

    log::info!("Writing final protocol output");
    let (cmd_ids, final_names) = output::finish_protobuf(
        &mut type_to_item,
        &proto_name_map,
        &rsp_notify_map,
        &req_map,
        &rsp_notify_names,
        &type_names,
        &field_names,
        out,
    )?;
    validate_packet_ids(&cmd_ids, &type_to_item, &request_scan.registry.all)?;
    serde_json::to_writer_pretty(&mut *cmdid_out, &cmd_ids)?;
    writeln!(cmdid_out)?;

    let original_names: HashMap<_, _> = proto_name_map
        .iter()
        .map(|(obf, name)| (name.as_str(), obf.as_str()))
        .collect();
    let defined_names: std::collections::HashSet<_> = type_to_item
        .values()
        .map(|item| match &*item.borrow() {
            output::ProtoItem::Message(m) => {
                m.deobfuscated_name.as_ref().unwrap_or(&m.name).clone()
            }
            output::ProtoItem::Enum(e) => e.deobfuscated_name.as_ref().unwrap_or(&e.name).clone(),
        })
        .collect();
    for handlers in [&mut cs_type_infos, &mut sc_packet_handlers] {
        let mut renamed = HashMap::<String, Vec<String>>::new();
        for (name, rvas) in handlers.drain() {
            let original = original_names.get(name.as_str()).copied().unwrap_or(&name);
            let name = final_names
                .get(original)
                .map(String::as_str)
                .unwrap_or(original)
                .to_owned();
            if defined_names.contains(&name) {
                renamed.entry(name).or_default().extend(rvas);
            }
        }
        for rvas in renamed.values_mut() {
            rvas.sort_unstable();
            rvas.dedup();
        }
        *handlers = renamed;
    }
    // Keep the raw identities available without making packetIds point at absent definitions.
    let obfuscated_ids: HashMap<_, _> = type_to_item
        .values()
        .filter_map(|item| {
            let item = item.borrow();
            match &*item {
                output::ProtoItem::Message(message) if message.cmd_id != 0 => {
                    Some((i32::from(message.cmd_id), message.name.clone()))
                }
                _ => None,
            }
        })
        .collect();
    let mut raw_ids = File::create("DUMP/packetIds.obfuscated.json.partial")?;
    serde_json::to_writer_pretty(&mut raw_ids, &obfuscated_ids)?;
    raw_ids.sync_all()?;
    drop(raw_ids);
    std::fs::rename(
        "DUMP/packetIds.obfuscated.json.partial",
        "DUMP/packetIds.obfuscated.json",
    )?;

    for (name, value) in [
        ("cs-type-infos.json", &cs_type_infos),
        ("sc-packet-handlers.json", &sc_packet_handlers),
    ] {
        let mut file = std::fs::File::create(format!("DUMP/{name}.partial"))?;
        serde_json::to_writer_pretty(&mut file, value)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(format!("DUMP/{name}.partial"), format!("DUMP/{name}"))?;
    }
    log::info!(
        "Protocol output verified: {} message definitions, {} packet IDs in {:.2}s",
        minimal_info_map.len(),
        cmd_ids.len(),
        started.elapsed().as_secs_f64()
    );
    recovery.finish(&type_to_item, &final_names)?;
    Ok(())
}

fn validate_packet_ids(
    cmd_ids: &HashMap<i32, String>,
    items: &output::TypeToItemMap,
    registry: &HashMap<RuntimeType, u16>,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        cmd_ids.len() == registry.len(),
        "packet ID output is incomplete: {}/{}",
        cmd_ids.len(),
        registry.len()
    );
    for (&ty, &id) in registry {
        anyhow::ensure!(
            items.get(&ty).is_some_and(|item| matches!(&*item.borrow(), output::ProtoItem::Message(message) if message.cmd_id == id && cmd_ids.get(&i32::from(id)) == Some(message.deobfuscated_name.as_ref().unwrap_or(&message.name)))),
            "CmdId {id} did not reach the protobuf output: {}", ty.il_name()
        );
    }
    Ok(())
}
