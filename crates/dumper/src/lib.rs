use std::{
    any::Any,
    fs::File,
    io::{BufWriter, Write},
    panic::{AssertUnwindSafe, catch_unwind},
    time::Instant,
};

use anyhow::Context;
use castorice_config::{Config, OutputKind};
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
    Metadata,
}

impl TryFrom<OutputKind> for Stage {
    type Error = anyhow::Error;

    fn try_from(kind: OutputKind) -> Result<Self, Self::Error> {
        Ok(match kind {
            OutputKind::Proto => Self::Proto,
            OutputKind::Schema => Self::Schema,
            OutputKind::Excel => Self::Excel,
            OutputKind::Config => Self::Config,
            OutputKind::Textmap => Self::Textmap,
            OutputKind::Csharp => Self::Csharp,
            OutputKind::Script => Self::Script,
            OutputKind::ScriptV2 => Self::ScriptV2,
            OutputKind::Metadata => Self::Metadata,
        })
    }
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
            Self::Metadata => "metadata",
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
            "metadata" => Stage::Metadata,
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
    let config = Config::load()?;
    let stages = if let Ok(selection) = std::env::var("CASTORICE_DUMP") {
        log::info!("Using CASTORICE_DUMP override: {selection}");
        parse_stages(&selection)?
    } else {
        config
            .output
            .stages
            .iter()
            .copied()
            .map(Stage::try_from)
            .collect::<anyhow::Result<Vec<_>>>()?
    };
    log::info!(
        "Selected dump stages: {}",
        stages
            .iter()
            .map(|stage| stage.name())
            .collect::<Vec<_>>()
            .join(",")
    );
    ensure_output_directory()?;
    let mut report = DumpReport {
        client_version: il2cpp::profile::supported()?.version.clone(),
        stages: std::iter::once("runtime")
            .chain(stages.iter().map(|stage| stage.name()))
            .map(|name| StageReport {
                name,
                status: "pending",
                error: None,
            })
            .collect(),
    };
    if stages.iter().any(|stage| *stage != Stage::Metadata) {
        report.execute(0, runtime::init_default)?;
    } else {
        report.stages[0].status = "not_required";
        report.save()?;
    }
    for (index, stage) in stages.into_iter().enumerate() {
        report.execute(index + 1, || run_stage(stage))?;
    }
    Ok(())
}

fn run_stage(stage: Stage) -> anyhow::Result<()> {
    match stage {
        Stage::Proto => {
            let result = (|| -> anyhow::Result<()> {
                let mut proto = BufWriter::new(File::create("DUMP/StarRail.proto.partial")?);
                let mut packet_ids = BufWriter::new(File::create("DUMP/packetIds.json.partial")?);
                proto::dump(
                    &mut proto,
                    &mut packet_ids,
                    proto::ProtoDumpMode::NativeGetters,
                    false,
                )?;
                proto.flush()?;
                packet_ids.flush()?;
                proto.get_ref().sync_all()?;
                packet_ids.get_ref().sync_all()?;
                drop((proto, packet_ids));
                std::fs::rename("DUMP/StarRail.proto.partial", "DUMP/StarRail.proto")?;
                std::fs::rename("DUMP/packetIds.json.partial", "DUMP/packetIds.json")?;
                proto::recovery::publish()?;
                Ok(())
            })();
            if let Err(error) = &result {
                if let Err(report_error) = proto::recovery::record_failure(error) {
                    log::error!("Cannot record NT publication failure: {report_error:#}");
                }
            }
            result?;
        }
        Stage::Schema => parser_data::dump()?,
        Stage::Excel => res::dump_excel()?,
        Stage::Config => res::dump_config(),
        Stage::Textmap => res::dump_textmap()?,
        Stage::Csharp => csharp::dump(false)?,
        Stage::Script => script::dump()?,
        Stage::ScriptV2 => script_v2::dump()?,
        Stage::Metadata => {
            let (assembly, _) = il2cpp::profile::loaded_module_path("GameAssembly.dll")?;
            let game_dir = assembly
                .parent()
                .context("GameAssembly.dll has no parent directory")?;
            log::info!(
                "Exporting original metadata from {} to DUMP/Morax",
                game_dir.display()
            );
            morax::export::export_game(game_dir, std::path::Path::new("DUMP/Morax"))?;
        }
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
        Err(error) => anyhow::bail!(
            "native exception at {}: {error:?}; no further stages will run",
            utils::memory::describe_address(error.address() as usize)
        ),
    }
}

pub fn ensure_output_directory() -> anyhow::Result<()> {
    std::fs::create_dir_all("DUMP")?;
    Ok(())
}
