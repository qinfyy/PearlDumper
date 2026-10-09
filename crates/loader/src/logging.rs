use castorice_config::LoggingConfig;
use env_logger::{Builder, Target, WriteStyle};

struct Logger {
    console: env_logger::Logger,
    file: env_logger::Logger,
}

impl log::Log for Logger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        self.console.enabled(metadata) || self.file.enabled(metadata)
    }

    fn log(&self, record: &log::Record<'_>) {
        self.file.log(record);
        self.console.log(record);
    }

    fn flush(&self) {
        self.file.flush();
        self.console.flush();
    }
}

fn install(logger: Logger) -> anyhow::Result<()> {
    let level = logger.console.filter().max(logger.file.filter());
    log::set_boxed_logger(Box::new(logger))
        .map_err(|error| anyhow::anyhow!("cannot install logger: {error}"))?;
    log::set_max_level(level);
    Ok(())
}

pub fn init(config: &LoggingConfig) -> anyhow::Result<()> {
    use std::fs::OpenOptions;

    use anyhow::Context;

    dumper::ensure_output_directory()?;
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open("DUMP/dump.log")
        .context("opening DUMP/dump.log")?;

    #[cfg(windows)]
    let ansi_error = ansi_term::enable_ansi_support().err();
    #[cfg(not(windows))]
    let ansi_error: Option<u32> = None;

    let mut console = Builder::new();
    console.filter_level(config.console_level);
    if let Ok(style) = std::env::var("RUST_LOG_STYLE") {
        console.parse_write_style(&style);
    }
    if ansi_error.is_some() {
        console.write_style(WriteStyle::Never);
    }
    let file = Builder::new()
        .filter_level(config.file_level)
        .write_style(WriteStyle::Never)
        .target(Target::Pipe(Box::new(file)))
        .build();
    install(Logger {
        console: console.build(),
        file,
    })?;
    if let Some(error) = ansi_error {
        tracing::warn!(
            "ANSI console support unavailable (Windows error {error}); using plain text"
        );
    }
    Ok(())
}
