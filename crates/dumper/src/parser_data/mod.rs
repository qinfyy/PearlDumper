use std::{fs::File, sync::OnceLock};

use anyhow::Context;
use il2cpp::{api, get_cached_class};

mod data_json;
mod excel_mod_rs;
mod field_reorderer;
mod typeindex;
mod util;

struct BinaryLayout {
    reader: String,
    from_binary: String,
}

static BINARY_LAYOUT: OnceLock<BinaryLayout> = OnceLock::new();

fn binary_layout() -> &'static BinaryLayout {
    BINARY_LAYOUT
        .get()
        .expect("schema layout must be initialized before export")
}

fn initialize_binary_layout() -> anyhow::Result<()> {
    if BINARY_LAYOUT.get().is_some() {
        return Ok(());
    }
    let class = get_cached_class("RPG.Client.LockTargetParam")
        .context("missing LockTargetParam schema anchor")?;
    let mut candidates = Vec::new();
    for method in class.get_methods() {
        if api::il2cpp_method_get_param_count(method) != 2 {
            continue;
        }
        let reader = api::il2cpp_method_get_param(method, 0);
        let output = api::il2cpp_method_get_param(method, 1);
        // Deserialization takes LockTargetParam&, while serialization takes the value.
        // Constructors and metadata order are not part of this binding.
        if output.il_name() != "RPG.Client.LockTargetParam&" {
            continue;
        }
        let reader_class = reader.get_class();
        let buffer_count = reader_class
            .get_fields()
            .iter()
            .filter(|&&field| api::il2cpp_field_get_type(field).il_name() == "System.Byte[]")
            .count();
        if buffer_count != 1 {
            continue;
        }
        let rva = method.checked_rva()?;
        log::info!(
            "Schema reader: {}::{}({}, LockTargetParam&) at RVA 0x{rva:X}",
            class.byval_arg().il_name(),
            method.get_name(),
            reader.il_name()
        );
        candidates.push(BinaryLayout {
            reader: reader.il_name().into_owned(),
            from_binary: method.get_name().into_owned(),
        });
    }
    anyhow::ensure!(
        candidates.len() == 1,
        "expected one schema deserializer, found {}",
        candidates.len()
    );
    BINARY_LAYOUT
        .set(candidates.pop().unwrap())
        .map_err(|_| anyhow::anyhow!("schema layout initialized twice"))?;
    Ok(())
}

#[allow(unused)]
pub fn dump() -> anyhow::Result<()> {
    initialize_binary_layout()?;
    log::info!("Generating HSR-OWNER schema: data.json, excel_paths.json, mod.rs");
    let mut structs = File::create("DUMP/mod.rs.partial")?;
    let mut data = File::create("DUMP/data.json.partial")?;
    let mut paths = File::create("DUMP/excel_paths.json.partial")?;
    log::info!("Schema step 1/2: generating mod.rs");
    excel_mod_rs::gen_excel_structs(&mut structs)?;
    log::info!("Schema step 2/2: generating data.json and excel_paths.json");
    data_json::gen_types(&mut data, &mut paths)?;
    log::info!("Flushing and publishing HSR-OWNER schema files");
    for file in [&structs, &data, &paths] {
        file.sync_all()?;
    }
    drop((structs, data, paths));
    for name in ["mod.rs", "data.json", "excel_paths.json"] {
        std::fs::rename(format!("DUMP/{name}.partial"), format!("DUMP/{name}"))?;
    }
    Ok(())
}
