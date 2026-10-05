use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, ensure};
use serde::Serialize;

use crate::{Metadata, dummydll::DummyDllReport, script::ScriptReport};

#[derive(Serialize)]
pub struct StageReport {
    name: &'static str,
    status: &'static str,
    elapsed_seconds: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Serialize)]
pub struct ExportReport {
    run_id: String,
    source_directory: PathBuf,
    status: &'static str,
    stages: Vec<StageReport>,
    images: usize,
    types: usize,
    field_default_records: u32,
    parameter_default_records: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    script: Option<ScriptReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dummy_dll: Option<DummyDllReport>,
    limitations: Vec<&'static str>,
}

pub fn load_game(game_dir: &Path) -> Result<Metadata> {
    let metadata_dir = game_dir.join("StarRail_Data/il2cpp_data/Metadata");
    let global = metadata_dir.join("global-metadata.dat");
    Metadata::load(
        &game_dir.join("GameAssembly.dll"),
        fs::read(&global).with_context(|| format!("reading {}", global.display()))?,
        &metadata_dir.join("startup-metadata.dat"),
    )
    .context("morax metadata load")
}

pub fn export_game(game_dir: &Path, output: &Path) -> Result<ExportReport> {
    fs::create_dir_all(output)?;
    let mut report = ExportReport {
        run_id: format!(
            "{}-{}",
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos(),
            std::process::id()
        ),
        source_directory: std::path::absolute(game_dir)?,
        status: "running",
        stages: [
            "metadata",
            "dump.cs",
            "script.json",
            "il2cpp.h",
            "stringLiterals.json",
            "DummyDll",
        ]
        .into_iter()
        .map(|name| StageReport {
            name,
            status: "pending",
            elapsed_seconds: 0.0,
            error: None,
        })
        .collect(),
        images: 0,
        types: 0,
        field_default_records: 0,
        parameter_default_records: 0,
        script: None,
        dummy_dll: None,
        limitations: vec![
            "Completed means selected offline artifacts were written, not a complete runtime dump.",
            "DummyDll method bodies are stubs; initialized field data, original custom attributes, generic constraints and marshal descriptors are not reconstructed.",
            "Native structure layouts and generic ABI signatures need separate validation; il2cpp.h is not a verified runtime ABI.",
            "Stripped parameter names remain empty in DummyDll and use pN placeholders in dump.cs.",
        ],
    };
    let metadata = report.execute(output, 0, |_| load_game(game_dir))?;
    report.images = metadata.images.len();
    report.types = metadata.types.len();
    report.field_default_records = metadata.header.field_default_values_count;
    report.parameter_default_records = metadata.header.parameter_default_values_count;
    report.execute(output, 1, |_| {
        publish_file(
            output,
            "dump.cs",
            crate::dump::build_dump_cs(&metadata)?.as_bytes(),
        )
    })?;
    report.execute(output, 2, |report| {
        let (json, stats) = crate::script::build_script_json_with_report(&metadata)?;
        publish_file(output, "script.json", json.as_bytes())?;
        report.script = Some(stats);
        Ok(())
    })?;
    report.execute(output, 3, |_| {
        publish_file(
            output,
            "il2cpp.h",
            crate::il2cpp_header::build_il2cpp_h(&metadata)?.as_bytes(),
        )
    })?;
    report.execute(output, 4, |_| {
        publish_file(
            output,
            "stringLiterals.json",
            crate::script::build_string_literals(&metadata)?.as_bytes(),
        )
    })?;
    report.execute(output, 5, |report| {
        let staged = output.join(format!("DummyDll.partial-{}", report.run_id));
        fs::create_dir(&staged)?;
        let stats = crate::dummydll::build_dummy_dll_with_report(&metadata, &staged)?;
        ensure!(
            stats.parameter_constants == metadata.parameter_default_values.len(),
            "only {} of {} parameter default records were emitted",
            stats.parameter_constants,
            metadata.parameter_default_values.len()
        );
        let target = output.join("DummyDll");
        let previous = output.join(format!("DummyDll.previous-{}", report.run_id));
        let had_previous = target.try_exists()?;
        if had_previous {
            fs::rename(&target, &previous)?;
        }
        if let Err(error) = fs::rename(&staged, &target) {
            if had_previous {
                fs::rename(&previous, &target)
                    .context("restoring previous DummyDll after failed publication")?;
            }
            return Err(error.into());
        }
        report.dummy_dll = Some(stats);
        Ok(())
    })?;
    report.status = "completed";
    report.save(output)?;
    Ok(report)
}

impl ExportReport {
    fn save(&self, output: &Path) -> Result<()> {
        publish_file(
            output,
            "morax-report.json",
            &serde_json::to_vec_pretty(self)?,
        )
    }

    fn execute<T>(
        &mut self,
        output: &Path,
        index: usize,
        work: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        self.stages[index].status = "running";
        self.save(output)?;
        println!("Starting {}", self.stages[index].name);
        let started = Instant::now();
        let result =
            work(self).with_context(|| format!("{} export failed", self.stages[index].name));
        self.stages[index].elapsed_seconds = started.elapsed().as_secs_f64();
        match &result {
            Ok(_) => self.stages[index].status = "completed",
            Err(error) => {
                self.status = "failed";
                self.stages[index].status = "failed";
                self.stages[index].error = Some(format!("{error:#}"));
            }
        }
        self.save(output)?;
        result
    }
}

fn publish_file(output: &Path, name: &str, bytes: &[u8]) -> Result<()> {
    let partial = output.join(format!("{name}.partial"));
    let mut file = fs::File::create(&partial)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&partial, output.join(name)).with_context(|| format!("publishing {name}"))
}
