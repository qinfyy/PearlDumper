use std::collections::{HashMap, HashSet};

use rayon::prelude::*;
use serde::Serialize;

use crate::crypt::generic_table;
use crate::crypt::script as script_key;
use crate::error::Result;
use crate::metadata::Metadata;
use crate::pe::{read_i32, read_u16, read_u32, read_u64};
use crate::type_def::strip_generic_arity;

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct ScriptMethod {
    address: u64,
    name: String,
    signature: String,
    type_signature: String,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct ScriptString {
    address: u64,
    value: String,
    #[serde(skip)]
    pub(crate) kind: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct ScriptMetadata {
    address: u64,
    name: String,
    signature: String,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct ScriptMetadataMethod {
    address: u64,
    name: String,
    method_address: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct ScriptMethodInvoker {
    address: u64,
    name: String,
    signature: String,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct StringLiteral {
    value: String,
    address: String,
}

#[derive(Serialize)]
struct ScriptJson {
    #[serde(rename = "ScriptMethod")]
    script_method: Vec<ScriptMethod>,
    #[serde(rename = "ScriptString")]
    script_string: Vec<ScriptString>,
    #[serde(rename = "ScriptMetadata")]
    script_metadata: Vec<ScriptMetadata>,
    #[serde(rename = "ScriptMetadataMethod")]
    script_metadata_method: Vec<ScriptMetadataMethod>,
    #[serde(rename = "MethodInvokers")]
    method_invokers: Vec<ScriptMethodInvoker>,
    #[serde(rename = "Addresses")]
    addresses: Vec<u64>,
    #[serde(rename = "UsageCoverage")]
    usage_coverage: UsageCoverage,
}

#[derive(Clone, Default, Serialize)]
pub struct UsageCoverage {
    pub pairs_per_kind: [u32; 8],
    pub slots_per_kind: [u32; 8],
    pub duplicates_per_kind: [u32; 8],
    pub ignored_pairs: u32,
}

#[derive(Serialize)]
pub struct ScriptReport {
    pub methods: usize,
    pub unique_method_addresses: usize,
    pub metadata: usize,
    pub metadata_methods: usize,
    pub strings: usize,
    pub invokers: usize,
    pub usage: UsageCoverage,
}

pub(crate) struct UsageSections {
    pub metadata: Vec<ScriptMetadata>,
    pub methods: Vec<ScriptMetadataMethod>,
    pub strings: Vec<ScriptString>,
    pub coverage: UsageCoverage,
}

#[derive(Clone, Copy)]
struct UsagePair {
    index: u32,
    kind: u32,
    source: u32,
    low: u32,
    address: u64,
}

enum UsageEntry {
    Metadata(ScriptMetadata),
    MetadataMethod(ScriptMetadataMethod),
    String(ScriptString),
}

struct GenericSpec {
    def_index: usize,
    declaring_type_index: usize,
    class_inst: i32,
    method_inst: i32,
    type_name: String,
    method_name: String,
    rva: u64,
}

pub struct GenericMethodInstance {
    pub def_index: usize,
    pub spec_index: u32,
    pub declaring_type_index: usize,
    pub class_inst: i32,
    pub method_inst: i32,
    pub type_name: String,
    pub method_name: String,
    pub rva: u64,
}

pub fn build_script_json(metadata: &Metadata) -> Result<String> {
    Ok(build_script_json_with_report(metadata)?.0)
}

pub fn build_script_json_with_report(metadata: &Metadata) -> Result<(String, ScriptReport)> {
    let method_to_type = build_method_to_type(metadata)?;
    let registry = metadata.build_struct_registry()?;

    let type_work: Vec<usize> = metadata
        .images()
        .iter()
        .flat_map(|image| image.type_start..image.type_start + image.type_count)
        .collect();
    let mut script_method: Vec<ScriptMethod> = type_work
        .par_iter()
        .map(|&type_index| -> Result<Vec<ScriptMethod>> {
            let type_def = metadata.type_def(type_index)?;
            let Some(method_start) = type_def.method_start else {
                return Ok(Vec::new());
            };
            let type_name = metadata.type_def_full_name(type_index)?;
            let mut methods = Vec::new();
            for local in 0..type_def.method_count {
                let method = metadata.method(method_start + local)?;
                if method.rva == 0 {
                    continue;
                }
                let (signature, type_signature) =
                    metadata.method_signature(method_start + local, type_index, &registry)?;
                methods.push(ScriptMethod {
                    address: method.rva,
                    name: format!("{type_name}$${}", method.name),
                    signature,
                    type_signature,
                });
            }
            Ok(methods)
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect();

    let generic_instances = enumerate_generic_methods(metadata)?;
    let mut generic_method_rva: HashMap<u32, u64> = HashMap::new();
    let mut seen_generic: HashSet<u64> = HashSet::new();
    for instance in &generic_instances {
        generic_method_rva
            .entry(instance.spec_index)
            .or_insert(instance.rva);
        if seen_generic.insert(instance.rva) {
            let (mut signature, type_signature) = metadata
                .method_signature(instance.def_index, instance.declaring_type_index, &registry)
                .map_err(|error| {
                    error.context(format!("generic signature spec {}", instance.spec_index))
                })?;
            if instance.is_method_generic()
                && let Some(prefix) = signature.strip_suffix("const MethodInfo* method);")
            {
                signature = format!("{prefix}const MethodInfo_{:X}* method);", instance.rva);
            }
            script_method.push(ScriptMethod {
                address: instance.rva,
                name: format!("{}$${}", instance.type_name, instance.method_name),
                signature,
                type_signature,
            });
        }
    }

    let usage = build_usage_sections(metadata, &method_to_type, &generic_method_rva)?;

    let method_invokers = build_method_invokers(metadata, &type_work)?;

    let mut addresses: Vec<u64> = script_method.iter().map(|method| method.address).collect();
    addresses.sort_unstable();
    addresses.dedup();

    let out = ScriptJson {
        script_method,
        script_string: usage.strings,
        script_metadata: usage.metadata,
        script_metadata_method: usage.methods,
        method_invokers,
        addresses,
        usage_coverage: usage.coverage,
    };
    let report = ScriptReport {
        methods: out.script_method.len(),
        unique_method_addresses: out.addresses.len(),
        metadata: out.script_metadata.len(),
        metadata_methods: out.script_metadata_method.len(),
        strings: out.script_string.len(),
        invokers: out.method_invokers.len(),
        usage: out.usage_coverage.clone(),
    };
    let json = serde_json::to_string_pretty(&out)
        .map_err(|error| format!("failed to serialize script.json: {error}"))?;
    Ok((json, report))
}

pub fn build_string_literals(metadata: &Metadata) -> Result<String> {
    let method_to_type = build_method_to_type(metadata)?;
    let usage = build_usage_sections(metadata, &method_to_type, &HashMap::new())?;
    let literals: Vec<StringLiteral> = usage
        .strings
        .into_iter()
        .filter(|entry| entry.kind == 5)
        .map(|entry| StringLiteral {
            value: entry.value,
            address: format!("0x{:X}", entry.address),
        })
        .collect();
    serde_json::to_string_pretty(&literals)
        .map_err(|error| format!("failed to serialize stringLiterals.json: {error}").into())
}

fn build_method_invokers(
    metadata: &Metadata,
    type_work: &[usize],
) -> Result<Vec<ScriptMethodInvoker>> {
    let per_type: Vec<Vec<(u64, String)>> = type_work
        .par_iter()
        .map(|&type_index| -> Result<Vec<(u64, String)>> {
            let type_def = metadata.type_def(type_index)?;
            let Some(method_start) = type_def.method_start else {
                return Ok(Vec::new());
            };
            let mut out = Vec::new();
            for local in 0..type_def.method_count {
                let method_index = method_start + local;
                if metadata.method(method_index)?.rva == 0 {
                    continue;
                }
                let Some(address) = metadata.method_invoker_rva(method_index)? else {
                    continue;
                };
                out.push((address, metadata.invoker_name(method_index)?));
            }
            Ok(out)
        })
        .collect::<Result<Vec<_>>>()?;

    let mut seen: HashSet<u64> = HashSet::new();
    let mut invokers: Vec<ScriptMethodInvoker> = Vec::new();
    for (address, name) in per_type.into_iter().flatten() {
        if seen.insert(address) {
            let signature = format!(
                "void* {name}(Il2CppMethodPointer pointer, const MethodInfo* methodMetadata, void* obj, void** args);"
            );
            invokers.push(ScriptMethodInvoker {
                address,
                name,
                signature,
            });
        }
    }
    invokers.sort_by_key(|invoker| invoker.address);
    Ok(invokers)
}

pub(crate) fn build_usage_sections(
    metadata: &Metadata,
    method_to_type: &[u32],
    generic_method_rva: &HashMap<u32, u64>,
) -> Result<UsageSections> {
    let pairs_count = usage_pairs_count(metadata)?;
    let payload = metadata.header.payload_offset as usize;
    let pairs_base = payload + metadata.header.usage_pairs_offset as usize;
    let method_specs_base = payload + metadata.header.method_specs_offset as usize;
    let method_count = method_to_type.len() as u32;

    let decoded: Vec<UsagePair> = (0..pairs_count)
        .into_par_iter()
        .map(|pair_index| -> Result<UsagePair> {
            let entry = pairs_base + pair_index as usize * 8;
            let low_raw = read_u32(&metadata.global_data, entry)?;
            let high_raw = read_u32(&metadata.global_data, entry + 4)?;
            let key = script_key::usage_pair_key(pair_index) as u32;
            let high = high_raw.wrapping_sub(key).wrapping_sub(script_key::PAIR_HIGH_SUB);
            let low = (low_raw ^ script_key::PAIR_LOW_XOR).wrapping_sub(key);
            let kind = high >> script_key::PAIR_KIND_SHIFT;
            let source = high & script_key::PAIR_SOURCE_MASK;
            let base = match kind {
                0 => metadata.cstring_usage_base_rva,
                1 => metadata.typeinfo_usage_base_rva,
                3 | 6 => metadata.method_usage_base_rva,
                4 => metadata.fieldinfo_usage_base_rva,
                5 => metadata.string_usage_base_rva,
                7 => metadata.runtime_type_usage_base_rva,
                _ => 0,
            };
            let slot = if matches!(kind, 0 | 7) { low & 0xFFFFFF } else { low };
            let address = u64::from(base) + 8 * u64::from(slot);
            if kind != 2 && !metadata.pe.contains_rva(address, 8) {
                return Err(format!("usage pair {pair_index} kind {kind} slot {slot} resolves outside image: 0x{address:X}").into());
            }
            Ok(UsagePair { index: pair_index, kind, source, low, address })
        })
        .collect::<Result<Vec<_>>>()?;

    let mut coverage = UsageCoverage::default();
    let mut seen: HashMap<u64, UsagePair> = HashMap::new();
    let mut to_resolve = Vec::new();
    for pair in decoded {
        let kind = pair.kind as usize;
        coverage.pairs_per_kind[kind] += 1;
        if kind == 2 {
            // The original usage initializer has no case 2.
            coverage.ignored_pairs += 1;
            continue;
        }
        if let Some(previous) = seen.get(&pair.address) {
            if (previous.kind, previous.source, previous.low) != (pair.kind, pair.source, pair.low)
            {
                return Err(format!(
                    "conflicting usage pairs {} and {} at RVA 0x{:X}",
                    previous.index, pair.index, pair.address
                )
                .into());
            }
            coverage.duplicates_per_kind[kind] += 1;
        } else {
            seen.insert(pair.address, pair);
            coverage.slots_per_kind[kind] += 1;
            to_resolve.push(pair);
        }
    }

    let resolved: Vec<UsageEntry> = to_resolve
        .par_iter()
        .map(|pair| -> Result<UsageEntry> {
            let UsagePair {
                kind,
                source,
                low,
                address,
                ..
            } = *pair;
            let resolve = || -> Result<UsageEntry> {
                Ok(match kind {
                    0 => UsageEntry::String(ScriptString {
                        address,
                        value: metadata.decode_string((low & 0xFF000000) | source)?,
                        kind,
                    }),
                    1 => {
                        let name = metadata.type_name(source, false)?;
                        UsageEntry::Metadata(ScriptMetadata {
                            address,
                            name: format!("{name}_TypeInfo"),
                            signature: format!("{name}_c*"),
                        })
                    }
                    3 => {
                        let type_index = method_to_type
                            .get(source as usize)
                            .copied()
                            .filter(|&index| index != u32::MAX)
                            .ok_or_else(|| {
                                format!("method definition {source} has no declaring type")
                            })?;
                        let type_name = metadata.type_def_full_name(type_index as usize)?;
                        let method = metadata.method(source as usize)?;
                        UsageEntry::MetadataMethod(ScriptMetadataMethod {
                            address,
                            name: format!("Method${type_name}.{}()", method.name),
                            method_address: method.rva,
                        })
                    }
                    4 => UsageEntry::Metadata(ScriptMetadata {
                        address,
                        name: resolve_field_ref_name(metadata, source)?,
                        signature: "FieldInfo*".to_owned(),
                    }),
                    5 => {
                        let value = metadata.decode_string_literal(source)?;
                        UsageEntry::String(ScriptString {
                            address,
                            value,
                            kind,
                        })
                    }
                    6 => {
                        let spec = resolve_generic_spec(
                            metadata,
                            method_specs_base,
                            source,
                            method_count,
                            method_to_type,
                        )?;
                        UsageEntry::MetadataMethod(ScriptMetadataMethod {
                            address,
                            name: format!("Method${}.{}()", spec.type_name, spec.method_name),
                            method_address: generic_method_rva
                                .get(&source)
                                .copied()
                                .unwrap_or(spec.rva),
                        })
                    }
                    7 => {
                        let name = metadata.type_name(source, false)?;
                        UsageEntry::Metadata(ScriptMetadata {
                            address,
                            name: format!("{name}_RuntimeType_{}", low >> 24),
                            // The client's mode-dependent wrapper is not a stock Il2CppType.
                            signature: "void*".to_owned(),
                        })
                    }
                    _ => return Err(format!("unsupported usage kind {kind}").into()),
                })
            };
            resolve().map_err(|error| {
                error.context(format!(
                    "usage pair {} kind {kind} source {source} RVA 0x{address:X}",
                    pair.index
                ))
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let mut script_metadata = Vec::new();
    let mut script_metadata_method = Vec::new();
    let mut script_string = Vec::new();
    for entry in resolved {
        match entry {
            UsageEntry::Metadata(m) => script_metadata.push(m),
            UsageEntry::MetadataMethod(m) => script_metadata_method.push(m),
            UsageEntry::String(s) => script_string.push(s),
        }
    }
    Ok(UsageSections {
        metadata: script_metadata,
        methods: script_metadata_method,
        strings: script_string,
        coverage,
    })
}

fn resolve_field_ref_name(metadata: &Metadata, source: u32) -> Result<String> {
    let entry = metadata.header.payload_offset as usize
        + metadata.header.field_refs_offset as usize
        + source as usize * 8;
    let key = script_key::field_ref_key(source);
    let type_index = key ^ read_u32(&metadata.global_data, entry + 4)?.wrapping_sub(388656835);
    let field_index =
        (key ^ read_u32(&metadata.global_data, entry)?.wrapping_sub(1800319632)) as usize;
    let type_entry = metadata.il2cpp_type(type_index)?;
    let type_name = metadata.type_name(type_index, false)?;
    let definition = match type_entry.kind {
        0x11 | 0x12 | 0x1C => type_entry.data as usize,
        0x15 => {
            metadata
                .generic_class_entry(type_entry.data)?
                .ok_or("field reference has no generic class definition")?
                .0
        }
        _ => metadata
            .types
            .iter()
            .position(|definition| {
                definition.declaring_type_index.is_none()
                    && format!("{}.{}", definition.namespace, definition.name) == type_name
            })
            .ok_or_else(|| {
                format!("field reference type {type_index} ({type_name}) has no definition")
            })?,
    };
    let type_def = metadata.type_def(definition)?;
    if field_index >= type_def.field_count {
        return Err(format!(
            "field reference local index {field_index} exceeds {type_name} fields {}",
            type_def.field_count
        )
        .into());
    }
    let field_start = type_def
        .field_start
        .ok_or("field reference type has no fields")?;
    let field_entry = metadata.header.payload_offset as usize
        + metadata.header.fields_offset as usize
        + (field_start + field_index) * crate::crypt::field::ENTRY_SIZE;
    let raw = crate::crypt::field::decrypt(
        &metadata.global_data,
        field_entry,
        field_start as u32,
        field_index as u32,
    )?;
    Ok(format!(
        "Field${type_name}.{}",
        metadata.decode_string(raw.name_index)?
    ))
}

impl GenericMethodInstance {
    pub fn is_method_generic(&self) -> bool {
        self.class_inst < 0 && self.method_inst >= 0
    }
}

pub(crate) fn enumerate_generic_methods(metadata: &Metadata) -> Result<Vec<GenericMethodInstance>> {
    let method_to_type = build_method_to_type(metadata)?;
    let method_count = method_to_type.len() as u32;
    let payload = metadata.header.payload_offset as usize;
    let method_specs_base = payload + metadata.header.method_specs_offset as usize;

    let table_base = payload + metadata.header.generic_method_table_offset as usize;
    let count = metadata.header.generic_method_table_count as usize;
    let primary = metadata.pe.pointer(
        metadata.code_registration_rva
            + generic_table::CODE_REGISTRATION_GENERIC_METHOD_POINTERS_OFF,
    )?;
    let secondary = metadata.pe.pointer(
        metadata.code_registration_rva + generic_table::CODE_REGISTRATION_SECONDARY_POINTERS_OFF,
    )?;

    let instances = (0..count)
        .into_par_iter()
        .map(|index| -> Result<Option<GenericMethodInstance>> {
            let entry = table_base + index * generic_table::ENTRY_SIZE;
            let spec_index = read_u32(&metadata.global_data, entry)
                .map_err(|error| error.context(format!("generic table entry {index}")))?;
            let method_index = read_u16(
                &metadata.global_data,
                entry + generic_table::METHOD_INDEX_OFF,
            )
            .map_err(|error| error.context(format!("generic table entry {index} method index")))?;
            let fallback_index = read_i32(
                &metadata.global_data,
                entry + generic_table::FALLBACK_INDEX_OFF,
            )
            .map_err(|error| error.context(format!("generic table entry {index} fallback index")))?;
            let spec = resolve_generic_spec(
                metadata,
                method_specs_base,
                spec_index,
                method_count,
                &method_to_type,
            ).map_err(|error| error.context(format!("generic table entry {index}, spec {spec_index}")))?;
            let va = if method_index != generic_table::SHARED_METHOD_INDEX {
                metadata.pe.rd64(primary + u32::from(method_index) * 8)
            } else if fallback_index >= 0 {
                metadata.pe.rd64(secondary + fallback_index as u32 * 8)
            } else {
                return Ok(None);
            };
            let va = va.map_err(|error| error.context(format!(
                "generic table entry {index}, spec {spec_index}, method {method_index}, fallback {fallback_index}"
            )))?;
            if va == 0 {
                return Ok(None);
            }
            if va < metadata.image_base || !metadata.pe.contains_rva(va - metadata.image_base, 1) {
                return Err(format!("generic table entry {index} has invalid VA 0x{va:X}").into());
            }
            Ok(Some(GenericMethodInstance {
                def_index: spec.def_index,
                spec_index,
                declaring_type_index: spec.declaring_type_index,
                class_inst: spec.class_inst,
                method_inst: spec.method_inst,
                type_name: spec.type_name,
                method_name: spec.method_name,
                rva: va - metadata.image_base,
            }))
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect();
    Ok(instances)
}

fn resolve_generic_spec(
    metadata: &Metadata,
    method_specs_base: usize,
    source: u32,
    method_count: u32,
    method_to_type: &[u32],
) -> Result<GenericSpec> {
    let spec = method_specs_base + source as usize * script_key::METHOD_SPEC_ENTRY_SIZE;
    let method_def_index = read_u32(&metadata.global_data, spec)?;
    if method_def_index >= method_count || method_to_type[method_def_index as usize] == u32::MAX {
        return Err(format!(
            "generic spec {source} has invalid method definition {method_def_index}"
        )
        .into());
    }
    let method_inst = read_i32(
        &metadata.global_data,
        spec + script_key::METHOD_SPEC_METHOD_INST_OFF,
    )?;
    let class_inst = read_i32(
        &metadata.global_data,
        spec + script_key::METHOD_SPEC_CLASS_INST_OFF,
    )?;
    if method_inst < -1 || class_inst < -1 {
        return Err(format!(
            "generic spec {source} has invalid instantiations {class_inst}/{method_inst}"
        )
        .into());
    }

    let type_index = method_to_type[method_def_index as usize] as usize;
    let mut type_name = metadata.type_def_full_name(type_index)?;
    if class_inst >= 0 {
        let args = metadata.generic_inst_args(class_inst as u32, false)?;
        type_name = format!("{}<{}>", strip_generic_arity(&type_name), args.join(","));
    }
    let method = metadata.method(method_def_index as usize)?;
    let mut method_name = method.name.clone();
    if method_inst >= 0 {
        let args = metadata.generic_inst_args(method_inst as u32, false)?;
        method_name = format!("{method_name}<{}>", args.join(","));
    }
    Ok(GenericSpec {
        def_index: method_def_index as usize,
        declaring_type_index: type_index,
        class_inst,
        method_inst,
        type_name,
        method_name,
        rva: method.rva,
    })
}

pub fn collect_generic_instantiations(
    metadata: &Metadata,
) -> Result<HashMap<usize, Vec<(u64, String)>>> {
    let mut map: HashMap<usize, Vec<(u64, String)>> = HashMap::new();
    for instance in enumerate_generic_methods(metadata)? {
        map.entry(instance.def_index).or_default().push((
            instance.rva,
            format!("{}.{}", instance.type_name, instance.method_name),
        ));
    }
    for instantiations in map.values_mut() {
        instantiations.sort();
        instantiations.dedup();
    }
    Ok(map)
}

pub(crate) fn build_method_to_type(metadata: &Metadata) -> Result<Vec<u32>> {
    let mut method_to_type: Vec<u32> = Vec::new();
    for image in metadata.images() {
        for type_index in image.type_start..image.type_start + image.type_count {
            let type_def = metadata.type_def(type_index)?;
            let Some(method_start) = type_def.method_start else {
                continue;
            };
            let end = method_start + type_def.method_count;
            if end > method_to_type.len() {
                method_to_type.resize(end, u32::MAX);
            }
            method_to_type[method_start..end].fill(type_index as u32);
        }
    }
    Ok(method_to_type)
}

fn usage_pairs_count(metadata: &Metadata) -> Result<u32> {
    let payload = metadata.header.payload_offset as usize;
    let lists_base = payload + metadata.header.usage_lists_offset as usize;
    let lists_span = metadata
        .header
        .usage_pairs_offset
        .checked_sub(metadata.header.usage_lists_offset)
        .ok_or("usage pair table precedes usage lists")?;
    if lists_span < 8 || !lists_span.is_multiple_of(4) {
        return Err(format!("implausible usage-lists span 0x{lists_span:X}").into());
    }
    let lists_count = lists_span / 4 - 1;

    let block_start = |i: u32| -> Result<u32> {
        let raw = read_u32(&metadata.global_data, lists_base + i as usize * 4)?;
        Ok(raw
            .wrapping_add(script_key::usage_list_key(i))
            .wrapping_sub(script_key::LIST_BIAS))
    };

    let start0 = block_start(0)?;
    if start0 != 0 {
        return Err(format!(
            "usage-lists sanity check failed: block 0 starts at {start0}, expected 0"
        )
        .into());
    }
    let mut prev = 0u32;
    for i in 1..=lists_count {
        let start = block_start(i)?;
        if start < prev {
            return Err(format!(
                "usage-lists table is not monotonic at block {i} ({start} < {prev})"
            )
            .into());
        }
        prev = start;
    }
    Ok(prev)
}

impl Metadata {
    pub(crate) fn decode_string_literal(&self, literal_index: u32) -> Result<String> {
        let table = self.header.payload_offset as usize
            + self.header.string_literal_offset as usize
            + literal_index as usize * 4;
        let off_i = read_u32(&self.global_data, table)?
            .wrapping_sub(script_key::string_literal_key(literal_index));
        let off_next = read_u32(&self.global_data, table + 4)?.wrapping_sub(
            script_key::string_literal_key(literal_index.wrapping_add(1)),
        );
        let length = off_next.wrapping_sub(off_i) as usize;
        if length == 0 {
            return Ok(String::new());
        }
        if length > 0x10000 {
            return Err(format!(
                "string literal {literal_index} decoded an implausible length {length}"
            )
            .into());
        }

        let base = self
            .header
            .string_literal_data_offset
            .wrapping_sub(script_key::LIT_DATA_BASE_SUB) as i32 as i64;
        let off = off_i.wrapping_add(script_key::LIT_OFF_BIAS) as i32 as i64;
        let data_offset = i64::from(self.header.payload_offset) + base + off;
        let data_offset = usize::try_from(data_offset).map_err(|_| {
            format!(
                "string literal {literal_index} resolved a negative payload offset {data_offset}"
            )
        })?;
        let qword_count = length.div_ceil(8);
        let mut keystream = (script_key::LIT_SEED_MUL.wrapping_mul(literal_index as u64)
            ^ script_key::LIT_SEED_XOR)
            .wrapping_add(script_key::LIT_SEED_ADD);
        let mut out = Vec::with_capacity(qword_count * 8);
        for chunk_index in 0..qword_count {
            let encrypted = read_u64(&self.global_data, data_offset + chunk_index * 8)?;
            out.extend_from_slice(&(encrypted ^ keystream).to_le_bytes());
            keystream = keystream.wrapping_add(script_key::LIT_PAYLOAD_INCREMENT);
        }
        out.truncate(length);
        Ok(String::from_utf8_lossy(&out).into_owned())
    }
}
