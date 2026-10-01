use std::{
    any::Any,
    fs::File,
    panic::{AssertUnwindSafe, catch_unwind},
    time::Instant,
};

use anyhow::Context;
use serde::Serialize;

mod csharp;
mod parser_data;
pub mod proto;
mod res;
mod runtime;
mod script;
mod script_v2;
mod version;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Stage {
    Proto,
    Schema,
    Excel,
    Config,
    Textmap,
    Csharp,
    Script,
    ScriptV2,
}

impl Stage {
    fn name(self) -> &'static str {
        match self {
            Self::Proto => "proto",
            Self::Schema => "schema",
            Self::Excel => "excel",
            Self::Config => "config",
            Self::Textmap => "textmap",
            Self::Csharp => "csharp",
            Self::Script => "script",
            Self::ScriptV2 => "script_v2",
        }
    }
}

fn parse_stages(value: &str) -> anyhow::Result<Vec<Stage>> {
    let mut stages = Vec::new();
    for name in value
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        let stage = match name {
            "proto" => Stage::Proto,
            "schema" | "parser" => Stage::Schema,
            "excel" => Stage::Excel,
            "config" => Stage::Config,
            "textmap" => Stage::Textmap,
            "csharp" => Stage::Csharp,
            "script" => Stage::Script,
            "script_v2" => Stage::ScriptV2,
            _ => anyhow::bail!("unknown dump stage {name}"),
        };
        if !stages.contains(&stage) {
            stages.push(stage);
        }
    }
    anyhow::ensure!(!stages.is_empty(), "no dump stages selected");
    Ok(stages)
}

#[derive(Serialize)]
struct StageReport {
    name: &'static str,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Serialize)]
struct DumpReport {
    client_version: String,
    stages: Vec<StageReport>,
}

impl DumpReport {
    fn save(&self) -> anyhow::Result<()> {
        let mut file = File::create("DUMP/report.json.partial")?;
        serde_json::to_writer_pretty(&mut file, self)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename("DUMP/report.json.partial", "DUMP/report.json")?;
        Ok(())
    }

    fn execute(
        &mut self,
        index: usize,
        work: impl FnOnce() -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        self.stages[index].status = "running";
        self.save()?;
        let name = self.stages[index].name;
        let started = Instant::now();
        log::info!("Starting {name}");
        let result = guarded(work).with_context(|| format!("{name} stage failed"));
        match &result {
            Ok(()) => {
                self.stages[index].status = "completed";
                log::info!("Finished {name} in {:.2}s", started.elapsed().as_secs_f64());
            }
            Err(error) => {
                self.stages[index].status = "failed";
                self.stages[index].error = Some(format!("{error:#}"));
                log::error!("{error:#} (after {:.2}s)", started.elapsed().as_secs_f64());
            }
        }
        self.save()?;
        result
    }
}

pub fn run() -> anyhow::Result<()> {
    // Validate all options before making the first call into the game.
    let selection = std::env::var("CASTORICE_DUMP").unwrap_or_else(|_| "proto,schema,excel".into());
    let stages = parse_stages(&selection)?;
    log::info!("Selected dump stages: {selection}");
    ensure_output_directory()?;
    let mut report = DumpReport {
        client_version: il2cpp::profile::supported()?.version,
        stages: std::iter::once("runtime")
            .chain(stages.iter().map(|stage| stage.name()))
            .map(|name| StageReport {
                name,
                status: "pending",
                error: None,
            })
            .collect(),
    };
    report.execute(0, runtime::init_default)?;
    for (index, stage) in stages.into_iter().enumerate() {
        report.execute(index + 1, || run_stage(stage))?;
    }
    Ok(())
}

fn run_stage(stage: Stage) -> anyhow::Result<()> {
    match stage {
        Stage::Proto => {
            script::prepare_method_metadata()?;
            let mut proto = File::create("DUMP/StarRail.proto.partial")?;
            let mut packet_ids = File::create("DUMP/packetIds.json.partial")?;
            proto::dump(
                &mut proto,
                &mut packet_ids,
                proto::ProtoDumpMode::ClassFieldNumber,
                false,
            )?;
            proto.sync_all()?;
            packet_ids.sync_all()?;
            drop((proto, packet_ids));
            std::fs::rename("DUMP/StarRail.proto.partial", "DUMP/StarRail.proto")?;
            std::fs::rename("DUMP/packetIds.json.partial", "DUMP/packetIds.json")?;
        }
        Stage::Schema => parser_data::dump()?,
        Stage::Excel => res::dump_excel()?,
        Stage::Config => res::dump_config(),
        Stage::Textmap => res::dump_textmap()?,
        Stage::Csharp => csharp::dump(false)?,
        Stage::Script => script::dump()?,
        Stage::ScriptV2 => script_v2::dump()?,
    }
    Ok(())
}

pub(crate) fn panic_message(payload: Box<dyn Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else if let Some(message) = payload.downcast_ref::<&str>() {
        message.to_string()
    } else {
        "non-string panic payload".into()
    }
}

pub(crate) fn guarded<T>(work: impl FnOnce() -> anyhow::Result<T>) -> anyhow::Result<T> {
    let mut work = Some(work);
    match microseh::try_seh(|| {
        catch_unwind(AssertUnwindSafe(
            work.take().expect("work already consumed"),
        ))
    }) {
        Ok(Ok(result)) => result,
        Ok(Err(payload)) => anyhow::bail!("Rust panic: {}", panic_message(payload)),
        Err(error) => anyhow::bail!("native exception: {error:?}; no further stages will run"),
    }
}

pub fn ensure_output_directory() -> anyhow::Result<()> {
    std::fs::create_dir_all("DUMP")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_and_parser_select_the_owner_schema_once() {
        assert_eq!(
            parse_stages(" schema, parser, excel, schema ").unwrap(),
            vec![Stage::Schema, Stage::Excel]
        );
    }

    #[test]
    fn invalid_selection_fails_before_runtime_initialization() {
        assert!(parse_stages("schema,invalid").is_err());
        assert!(parse_stages(", ,").is_err());
    }

    #[test]
    fn guard_preserves_errors_and_panics() {
        assert_eq!(guarded(|| Ok(7)).unwrap(), 7);
        assert!(
            guarded::<()>(|| anyhow::bail!("test failure"))
                .unwrap_err()
                .to_string()
                .contains("test failure")
        );
        assert!(
            guarded::<()>(|| panic!("test panic"))
                .unwrap_err()
                .to_string()
                .contains("test panic")
        );
    }
}
