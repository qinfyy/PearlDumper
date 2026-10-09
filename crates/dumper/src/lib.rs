use std::{
    any::Any,
    fs::File,
    io::{BufWriter, Write},
    panic::{AssertUnwindSafe, catch_unwind},
    time::Instant,
};

use anyhow::Context;
use castorice_config::{Config, OutputKind};

mod csharp;
mod metadata_runtime;
pub mod metadata_scan;
mod parser_data;
pub mod proto;
mod runtime;
mod version;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Stage {
    Proto,
    Schema,
    Csharp,
}

impl TryFrom<OutputKind> for Stage {
    type Error = anyhow::Error;

    fn try_from(kind: OutputKind) -> Result<Self, Self::Error> {
        Ok(match kind {
            OutputKind::Proto => Self::Proto,
            OutputKind::Schema => Self::Schema,
            OutputKind::Csharp => Self::Csharp,
        })
    }
}

impl Stage {
    fn name(self) -> &'static str {
        match self {
            Self::Proto => "proto",
            Self::Schema => "schema",
            Self::Csharp => "csharp",
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
            "csharp" => Stage::Csharp,
            "excel" | "config" | "textmap" | "script" | "script_v2" => {
                anyhow::bail!(
                    "dump stage {name} was removed; supported stages: proto,schema,csharp"
                )
            }
            "metadata" => anyhow::bail!(
                "metadata is an independent offline tool; run morax.exe GAME_DIR OUTPUT_DIR"
            ),
            _ => anyhow::bail!("unknown dump stage {name}"),
        };
        if !stages.contains(&stage) {
            stages.push(stage);
        }
    }
    anyhow::ensure!(!stages.is_empty(), "no dump stages selected");
    Ok(stages)
}

fn execute_stage(name: &str, work: impl FnOnce() -> anyhow::Result<()>) -> anyhow::Result<()> {
    let started = Instant::now();
    log::info!("Starting {name}");
    let result = guarded(work).with_context(|| format!("{name} stage failed"));
    match &result {
        Ok(()) => {
            log::info!("Finished {name} in {:.2}s", started.elapsed().as_secs_f64());
        }
        Err(error) => {
            log::error!("{error:#} (after {:.2}s)", started.elapsed().as_secs_f64());
        }
    }
    result
}

pub fn run(config: &Config) -> anyhow::Result<()> {
    // Validate all options before making the first call into the game.
    let stages = if let Ok(selection) = std::env::var("DUMPER_STAGES") {
        log::info!("Using DUMPER_STAGES override: {selection}");
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
    execute_stage("runtime", runtime::init_default)?;
    for stage in stages {
        execute_stage(stage.name(), || run_stage(stage))?;
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
        Stage::Csharp => csharp::dump()?,
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
