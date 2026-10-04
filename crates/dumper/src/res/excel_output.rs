use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    time::{Duration, Instant},
};

use anyhow::Context;
use iced_x86::{Decoder, DecoderOptions, Mnemonic, OpKind};
use il2cpp::vm::{
    array::Il2CppArray,
    boxed_value::BoxedBool,
    method::Il2CppMethod,
    object::Il2CppObject,
    string::Il2CppString,
    value::{Il2CppValue, Void},
};
use il2cpp::{api, get_cached_class, get_native_method};
use reflection::{
    assembly, field_info::FieldInfo, method_info::MethodInfo, runtime_type::RuntimeType,
    serializer::BoxedSerializer,
};
use serde::Serialize;

#[derive(Default, Serialize)]
struct ExcelReport {
    expected_files: BTreeSet<String>,
    exported_files: BTreeMap<String, usize>,
    unconfigured_tables: Vec<String>,
    failures: Vec<ExcelFailure>,
}

#[derive(Serialize)]
struct ExcelFailure {
    table: String,
    path: Option<String>,
    error: String,
}

impl ExcelReport {
    fn fail(&mut self, table: RuntimeType, path: Option<&str>, error: anyhow::Error) {
        let table = table.il_name().into_owned();
        log::warn!("Excel table {table}, path={path:?}: {error:#}");
        self.failures.push(ExcelFailure {
            table,
            path: path.map(str::to_owned),
            error: format!("{error:#}"),
        });
    }
}

pub fn dump() -> anyhow::Result<()> {
    std::fs::create_dir_all("DUMP/Resources/ExcelOutput")?;
    let assemblies = assembly::get_assemblies();
    let assembly = assemblies
        .iter()
        .find(|assembly| assembly.get_name() == "RPG.GameCore.Config")
        .context("RPG.GameCore.Config assembly not found")?;
    let mut report = ExcelReport::default();
    let types = assembly.get_types();
    let mut progress = Instant::now();
    let mut resources_ready = false;
    for (index, &table) in types.iter().enumerate() {
        if progress.elapsed() >= Duration::from_secs(2) {
            log::info!(
                "Excel: {index}/{} types, {} files exported",
                types.len(),
                report.exported_files.len()
            );
            progress = Instant::now();
        }
        if let Err(error) = dump_table(table, &mut report, &mut resources_ready)
            .with_context(|| format!("Excel table {}", table.il_name()))
        {
            report.fail(table, None, error);
            if !resources_ready {
                super::write_json(Path::new("DUMP/excel_report.json"), &report)?;
                anyhow::bail!("Excel initialization failed; see DUMP/excel_report.json");
            }
        }
    }
    super::write_json(Path::new("DUMP/excel_report.json"), &report)?;
    log::info!(
        "Exported {}/{} Excel files; {} failures, {} unconfigured tables",
        report.exported_files.len(),
        report.expected_files.len(),
        report.failures.len(),
        report.unconfigured_tables.len()
    );
    anyhow::ensure!(
        !report.exported_files.is_empty(),
        "no Excel tables were exported"
    );
    anyhow::ensure!(
        report.failures.is_empty() && report.exported_files.len() == report.expected_files.len(),
        "Excel output is incomplete: {}/{} files, {} failures; see DUMP/excel_report.json",
        report.exported_files.len(),
        report.expected_files.len(),
        report.failures.len()
    );
    Ok(())
}

fn dump_table(
    table: RuntimeType,
    report: &mut ExcelReport,
    resources_ready: &mut bool,
) -> anyhow::Result<()> {
    let fields = table.get_fields_il2cpp();
    let mut path_fields = Vec::new();
    let mut dictionary_types = BTreeSet::new();
    for field in fields {
        if !field.get_isstatic()?.unbox() {
            continue;
        }
        let name = field.get_field_type()?.format_type_name(true);
        if name == "string[]" {
            path_fields.push(field);
        } else if name.contains("Dictionary")
            && (name.contains("Row") || name.contains("CommonIndexKey"))
        {
            dictionary_types.insert(name);
        }
    }
    if dictionary_types.is_empty() || path_fields.is_empty() {
        return Ok(());
    }
    let factory = enumerator_factory(table)?;
    let path_field = if path_fields.len() == 1 {
        path_fields[0]
    } else {
        // Adapter tables also contain fullPathList, populated after loading.
        let load = table_loader(table, factory)?;
        let code = utils::scanner::game_assembly_function(load.checked_rva()?).unwrap_or(&[]);
        let candidates: Vec<_> = path_fields
            .iter()
            .copied()
            .filter(|field| {
                Decoder::new(64, code, DecoderOptions::NONE)
                    .into_iter()
                    .any(|instruction| {
                        instruction.mnemonic() == Mnemonic::Mov
                            && instruction.op1_kind() == OpKind::Memory
                            && !instruction.is_ip_rel_memory_operand()
                            && instruction.memory_displacement64() as usize == field.get_offset()
                    })
            })
            .collect();
        if candidates.len() == 1 {
            candidates[0]
        } else {
            let candidates: Vec<_> = path_fields
                .into_iter()
                .filter(|field| {
                    field.get_name().is_ok_and(|name| {
                        matches!(name.as_str().as_ref(), "s_PathList" | "JHPLPOEEIFE")
                    })
                })
                .collect();
            anyhow::ensure!(candidates.len() == 1, "ambiguous Excel source path list");
            candidates[0]
        }
    };
    let original_paths = path_field
        .get_value(Il2CppObject::NULL)
        .context("failed to read Excel path list")?;
    anyhow::ensure!(
        !original_paths.is_null(),
        "Excel path list is not initialized"
    );
    let paths = Il2CppArray(original_paths.0)
        .to_vec::<Il2CppString>()
        .into_iter()
        .filter(|path| !path.is_null())
        .map(|path| path.as_str().into_owned())
        .filter(|path| !path.is_empty())
        .collect::<Vec<_>>();
    if paths.is_empty() {
        report
            .unconfigured_tables
            .push(table.il_name().into_owned());
        return Ok(());
    }

    let mut groups = BTreeMap::<String, Vec<String>>::new();
    for path in &paths {
        let name = output_name(path)?.to_owned();
        report.expected_files.insert(name.clone());
        groups.entry(name).or_default().push(path.clone());
    }
    if !*resources_ready {
        wait_for_resources(&paths[0])?;
        *resources_ready = true;
    }
    // A single logical file can have client and GameCore layers. Keep their order.
    // Split distinct files only; otherwise use the factory's own lazy loader.
    let load = (groups.len() > 1)
        .then(|| table_loader(table, factory))
        .transpose()?;
    for (name, paths) in &groups {
        if report.exported_files.contains_key(name) {
            log::debug!("Skipping already exported Excel output {name}");
            continue;
        }
        let result = (|| {
            if let Some(load) = load {
                set_paths(path_field, paths)?;
                invoke_load(load)?;
            }
            let rows = collect_rows(table)?;
            super::write_json(
                &Path::new("DUMP/Resources/ExcelOutput").join(format!("{name}.json")),
                &rows,
            )?;
            log::debug!("Exported {name}: {} rows", rows.len());
            report.exported_files.insert(name.clone(), rows.len());
            Ok(())
        })();
        if let Err(error) = result {
            report.fail(table, Some(&paths.join("; ")), error);
        }
    }
    if let Some(load) = load {
        path_field.set_value(Il2CppObject::NULL, original_paths)?;
        invoke_load(load).context("restoring original Excel table paths and data")?;
    }
    Ok(())
}

fn wait_for_resources(path: &str) -> anyhow::Result<()> {
    let exists = get_native_method("RPG.Client.AssetLoader::ExistsDesignData(System.String)")
        .context("ExistsDesignData method not found")?;
    let started = Instant::now();
    // ExistsDesignData calls JsonToBinaryPath, which adds BakedConfig/ and replaces the extension.
    let logical_path = path.replace('\\', "/");
    let logical_path = logical_path
        .strip_prefix("BakedConfig/")
        .unwrap_or(&logical_path);
    let logical_path = logical_path.strip_suffix(".bytes").unwrap_or(logical_path);
    let probe_path = if logical_path.ends_with(".json") {
        logical_path.to_owned()
    } else {
        format!("{logical_path}.json")
    };
    let path_string = Il2CppString::from(probe_path.as_str());
    loop {
        match exists.invoke::<BoxedBool>(Il2CppObject::NULL, &[&path_string]) {
            Ok(value) if !value.is_null() && value.unbox() => return Ok(()),
            Err(error) => log::debug!("Design data probe {probe_path}: {error:?}"),
            _ => {}
        }
        anyhow::ensure!(
            started.elapsed() < Duration::from_secs(120),
            "design data is not available: {path}; finish the game's resource loading before dumping Excel"
        );
        log::info!(
            "Waiting for design data {path} ({:.0}s)",
            started.elapsed().as_secs_f64()
        );
        std::thread::sleep(Duration::from_secs(2));
    }
}

fn set_paths(field: FieldInfo, paths: &[String]) -> anyhow::Result<()> {
    let string_class = get_cached_class("System.String").context("missing System.String")?;
    let mut array = Il2CppArray::new(string_class.get_array_class(1), paths.len());
    for (item, path) in array.as_mut_slice::<Il2CppString>().iter_mut().zip(paths) {
        *item = Il2CppString::from(path.as_str());
    }
    field.set_value(Il2CppObject::NULL, Il2CppObject(array.0))?;
    Ok(())
}

fn invoke_load(load: Il2CppMethod) -> anyhow::Result<()> {
    let rva = load.checked_rva()?;
    load.invoke::<Void>(Il2CppObject::NULL, &[])
        .map_err(|error| anyhow::anyhow!("Il2CppException: {error:?}"))
        .with_context(|| format!("Excel loader {} at RVA 0x{rva:X}", load.get_name()))?;
    Ok(())
}

fn table_loader(table: RuntimeType, factory: MethodInfo) -> anyhow::Result<Il2CppMethod> {
    let rva = factory.get_il2cpp_method().checked_rva()?;
    let code = utils::scanner::game_assembly_function(rva)?;
    let calls: BTreeSet<_> = Decoder::with_ip(64, code, rva as u64, DecoderOptions::NONE)
        .into_iter()
        .filter(|instruction| {
            instruction.mnemonic() == Mnemonic::Call
                && instruction.op0_kind() == OpKind::NearBranch64
        })
        .map(|instruction| instruction.near_branch_target() as usize)
        .collect();
    // LoadData clears old rows itself. Its factory call survives method-name/flag obfuscation.
    let candidates: Vec<_> = table
        .get_il2cpp_type()
        .get_class()
        .get_methods()
        .into_iter()
        .filter(|method| {
            api::il2cpp_method_get_param_count(*method) == 0
                && !method.get_name().starts_with('.')
                && api::il2cpp_method_get_return_type(*method).il_name() == "System.Void"
                && method.checked_rva().is_ok_and(|rva| calls.contains(&rva))
        })
        .collect();
    if candidates.len() == 1 {
        return Ok(candidates[0]);
    }
    let named = table
        .get_methods_il2cpp()
        .into_iter()
        .filter(|method| {
            method
                .get_name()
                .is_ok_and(|name| matches!(name.as_str().as_ref(), "LoadData" | "FCMDLFKMBMH"))
                && method.get_parameters().is_empty()
                && method
                    .get_return_type()
                    .is_ok_and(|ty| ty.il_name() == "System.Void")
        })
        .collect::<Vec<_>>();
    anyhow::ensure!(
        named.len() == 1,
        "ambiguous Excel loader: {} factory calls, {} named candidates",
        candidates.len(),
        named.len()
    );
    Ok(named[0].get_il2cpp_method())
}

fn enumerator_factory(table: RuntimeType) -> anyhow::Result<MethodInfo> {
    let methods = table
        .get_methods_il2cpp()
        .into_iter()
        .filter(|method| {
            // The table has static storage but its method flags are obfuscated.
            // FixEnumerator reads eager tables; AdapterEnumerator reads LRU-backed tables.
            method.get_parameters().is_empty()
                && method.get_return_type().is_ok_and(|ty| {
                    ty.get_name().is_ok_and(|name| {
                        matches!(
                            name.as_str().as_ref(),
                            "FixEnumerator`2" | "AdapterEnumerator`2" | "AdapterEnumerator64`2"
                        )
                    })
                })
        })
        .collect::<Vec<_>>();
    anyhow::ensure!(
        methods.len() == 1,
        "expected one parameterless row enumerator factory, found {}",
        methods.len()
    );
    Ok(methods[0])
}

pub(super) fn collect_rows(table: RuntimeType) -> anyhow::Result<Vec<serde_json::Value>> {
    let get_enumerator = enumerator_factory(table)?;
    let rva = get_enumerator.get_il2cpp_method().checked_rva()?;
    let enumerator_name = get_enumerator.get_name()?.as_str().into_owned();
    log::debug!(
        "Excel enumerator {}::{} at RVA 0x{rva:X}",
        table.il_name(),
        enumerator_name
    );
    let enumerator_type = get_enumerator.get_return_type()?;
    let current = enumerator_type.get_property("Current".into(), 62)?;
    anyhow::ensure!(
        !current.is_null(),
        "table enumerator has no Current property"
    );
    let row_type = current.get_property_type()?;
    let enumerator = get_enumerator
        .get_il2cpp_method()
        .invoke::<Il2CppObject>(Il2CppObject::NULL, &[])
        .map_err(|error| anyhow::anyhow!("Il2CppException: {error:?}"))
        .with_context(|| {
            format!(
                "Excel enumerator factory {}::{} at RVA 0x{rva:X}",
                table.il_name(),
                enumerator_name
            )
        })?;
    anyhow::ensure!(
        !enumerator.is_null(),
        "Excel enumerator factory {}::{} returned null",
        table.il_name(),
        enumerator_name
    );
    let runtime_type = RuntimeType::from_object(enumerator)?;
    let count = enumerator_type.get_property("Count".into(), 62)?;
    anyhow::ensure!(!count.is_null(), "table enumerator has no Count property");
    let expected_rows = count.get_value(enumerator)?;
    anyhow::ensure!(
        !expected_rows.is_null(),
        "table enumerator Count returned null"
    );
    let expected_rows = expected_rows.unbox::<i32>();
    anyhow::ensure!(
        (0..=2_000_000).contains(&expected_rows),
        "invalid Excel row count {expected_rows}"
    );
    let is_value_type = runtime_type.get_isvaluetype()?.unbox();
    let move_next = runtime_type
        .find_method_specific("MoveNext", 62, &[])
        .context("table enumerator has no parameterless MoveNext")?
        .get_il2cpp_method();
    let move_next_rva = move_next.checked_rva()?;
    let receiver = if is_value_type {
        let payload = il2cpp::api::il2cpp_object_unbox(enumerator) as usize;
        anyhow::ensure!(payload != 0, "value type enumerator unboxed to null");
        Il2CppObject(payload)
    } else {
        enumerator
    };
    log::debug!(
        "Excel enumerator object=0x{:X}, class=0x{:X}, value_type={is_value_type}, receiver=0x{:X}, MoveNext RVA=0x{move_next_rva:X}, row={}",
        enumerator.0,
        enumerator.get_class().0,
        receiver.0,
        row_type.il_name()
    );
    let mut serializer = BoxedSerializer::default();
    let mut rows = Vec::new();
    while move_next
        .invoke::<BoxedBool>(receiver, &[])
        .map_err(|error| anyhow::anyhow!("Il2CppException: {error:?}"))
        .with_context(|| {
            format!(
                "Excel MoveNext {}::{} at RVA 0x{move_next_rva:X} after {} rows",
                table.il_name(),
                move_next.get_name(),
                rows.len()
            )
        })?
        .unbox()
    {
        anyhow::ensure!(
            rows.len() < 2_000_000,
            "table enumeration exceeded the row limit"
        );
        let row = current.get_value(enumerator).with_context(|| {
            format!(
                "Excel Current {}::Current after {} rows",
                table.il_name(),
                rows.len()
            )
        })?;
        anyhow::ensure!(
            !row.is_null(),
            "Excel Current returned null after {} rows",
            rows.len()
        );
        rows.push(serializer.serialize(row_type, row).with_context(|| {
            format!(
                "Excel row serialization {}::Current at row {}",
                table.il_name(),
                rows.len()
            )
        })?);
    }
    anyhow::ensure!(
        rows.len() == expected_rows as usize,
        "Excel row coverage mismatch: {}/{expected_rows}",
        rows.len()
    );
    Ok(rows)
}

fn output_name(path: &str) -> anyhow::Result<&str> {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let name = name.strip_suffix(".bytes").unwrap_or(name);
    anyhow::ensure!(
        !name.is_empty()
            && name != "."
            && name != ".."
            && !name
                .chars()
                .any(|ch| ch.is_control() || "<>:\"|?*".contains(ch)),
        "invalid Excel output path {path}"
    );
    Ok(name)
}
