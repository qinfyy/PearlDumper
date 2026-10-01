use std::path::Path;

use anyhow::Context;
use il2cpp::get_cached_class;
use reflection::runtime_type::RuntimeType;

pub fn dump() -> anyhow::Result<()> {
    let table = RuntimeType::from_class(
        get_cached_class("RPG.GameCore.TextmapExcelTable")
            .context("TextmapExcelTable not found")?,
    )?;
    let rows = super::excel_output::collect_rows(table)?;
    super::write_json(Path::new("DUMP/Resources/TextMap/TextMapEN.json"), &rows)?;
    log::info!("Exported {} textmap rows", rows.len());
    Ok(())
}
