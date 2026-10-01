use std::{collections::BTreeSet, path::Path};

use anyhow::Context;
use il2cpp::vm::{
    array::Il2CppArray, boxed_value::BoxedBool, object::Il2CppObject, string::Il2CppString,
    value::Il2CppValue,
};
use reflection::{assembly, runtime_type::RuntimeType, serializer::BoxedSerializer};

pub fn dump() -> anyhow::Result<()> {
    std::fs::create_dir_all("DUMP/Resources/ExcelOutput")?;
    let assemblies = assembly::get_assemblies();
    let assembly = assemblies
        .iter()
        .find(|assembly| assembly.get_name() == "RPG.GameCore.Config")
        .context("RPG.GameCore.Config assembly not found")?;
    let mut written = BTreeSet::new();
    for table in assembly.get_types() {
        dump_table(table, &mut written)
            .with_context(|| format!("Excel table {}", table.il_name()))?;
    }
    anyhow::ensure!(!written.is_empty(), "no Excel tables were exported");
    log::info!("Exported {} Excel files", written.len());
    Ok(())
}

fn dump_table(table: RuntimeType, written: &mut BTreeSet<String>) -> anyhow::Result<()> {
    let fields = table.get_fields_il2cpp();
    let mut path_fields = Vec::new();
    let mut has_dictionary = false;
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
            has_dictionary = true;
        }
    }
    if !has_dictionary || path_fields.is_empty() {
        return Ok(());
    }
    anyhow::ensure!(path_fields.len() == 1, "ambiguous Excel path list");
    let paths = path_fields[0].get_value(Il2CppObject::NULL)?;
    anyhow::ensure!(!paths.is_null(), "Excel path list is not initialized");
    let paths = Il2CppArray(paths.0)
        .to_vec::<Il2CppString>()
        .into_iter()
        .map(|path| path.as_str().into_owned())
        .collect::<BTreeSet<_>>();
    if paths.is_empty() {
        return Ok(());
    }

    let rows = collect_rows(table)?;
    for path in paths {
        let name = output_name(&path)?.to_string();
        anyhow::ensure!(
            written.insert(name.clone()),
            "duplicate Excel output name {name}"
        );
        super::write_json(
            &Path::new("DUMP/Resources/ExcelOutput").join(format!("{name}.json")),
            &rows,
        )?;
        log::debug!("Exported {name}: {} rows", rows.len());
    }
    Ok(())
}

pub(super) fn collect_rows(table: RuntimeType) -> anyhow::Result<Vec<serde_json::Value>> {
    let methods = table
        .get_methods_il2cpp()
        .into_iter()
        .filter(|method| {
            method.is_static()
                && method.get_parameters().is_empty()
                && method.get_return_type().is_ok_and(|ty| {
                    ty.get_name()
                        .is_ok_and(|name| name.as_str().contains("Enumerator"))
                })
        })
        .collect::<Vec<_>>();
    anyhow::ensure!(
        methods.len() == 1,
        "expected one parameterless static table enumerator, found {}",
        methods.len()
    );
    let get_enumerator = methods[0];
    let enumerator_type = get_enumerator.get_return_type()?;
    let current = enumerator_type.get_property("Current".into(), 62)?;
    anyhow::ensure!(
        !current.is_null(),
        "table enumerator has no Current property"
    );
    let row_type = current.get_property_type()?;
    let enumerator = get_enumerator
        .get_il2cpp_method()
        .invoke::<Il2CppObject>(Il2CppObject::NULL, &[])?;
    anyhow::ensure!(!enumerator.is_null(), "table enumerator is null");
    let runtime_type = RuntimeType::from_object(enumerator)?;
    let move_next = runtime_type
        .find_method_specific("MoveNext", 62, &[])
        .context("table enumerator has no parameterless MoveNext")?
        .get_il2cpp_method();
    let receiver = if runtime_type.get_isvaluetype()?.unbox() {
        Il2CppObject(il2cpp::api::il2cpp_object_unbox(enumerator) as usize)
    } else {
        enumerator
    };
    let mut serializer = BoxedSerializer::default();
    let mut rows = Vec::new();
    while move_next.invoke::<BoxedBool>(receiver, &[])?.unbox() {
        anyhow::ensure!(
            rows.len() < 2_000_000,
            "table enumeration exceeded the row limit"
        );
        let row = current.get_value(enumerator)?;
        rows.push(serializer.serialize(row_type, row)?);
    }
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

#[cfg(test)]
mod tests {
    #[test]
    fn output_paths_remain_inside_excel_directory() {
        assert_eq!(
            super::output_name("BakedConfig/ExcelOutput/Avatar.bytes").unwrap(),
            "Avatar"
        );
        assert_eq!(
            super::output_name(r"BakedConfig\ExcelOutput\Avatar.bytes").unwrap(),
            "Avatar"
        );
        assert!(super::output_name("..").is_err());
        assert!(super::output_name("Avatar:stream.bytes").is_err());
        assert!(super::output_name("ExcelOutput/").is_err());
    }
}
