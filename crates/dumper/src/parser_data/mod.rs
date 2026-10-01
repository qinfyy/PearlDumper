use std::{borrow::Cow, fs::File, sync::LazyLock};

use il2cpp::get_cached_class;
use reflection::runtime_type::RuntimeType;

mod data_json;
mod excel_mod_rs;
mod field_reorderer;
mod typeindex;
mod util;

static BINARY_READER_CLASS: LazyLock<Cow<'static, str>> = LazyLock::new(|| {
    let lock_target_param_type =
        RuntimeType::from_class(get_cached_class("RPG.Client.LockTargetParam").unwrap()).unwrap();

    let method = lock_target_param_type
        .get_methods(62)
        .into_iter()
        .next()
        .unwrap();

    let class_name = method
        .get_parameters()
        .first()
        .unwrap()
        .get_parameter_type()
        .unwrap()
        .il_name();
    log::debug!("[Parser Data] BinaryReader Class => {class_name}");

    class_name
});

static FROM_BINARY_FUNC_NAME: LazyLock<Cow<'static, str>> = LazyLock::new(|| {
    let lock_target_param_type =
        RuntimeType::from_class(get_cached_class("RPG.Client.LockTargetParam").unwrap()).unwrap();

    let method = lock_target_param_type
        .get_methods(62)
        .into_iter()
        .next()
        .unwrap();

    let method_name = method.get_name().unwrap().as_str();

    log::debug!("[Parser Data] FromBinary FuncName => {method_name}");

    method_name
});

#[allow(unused)]
pub fn dump() -> anyhow::Result<()> {
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
