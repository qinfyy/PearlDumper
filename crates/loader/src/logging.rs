use env_logger::{Builder, Target, WriteStyle};

const CONSOLE_FILTER: &str = "info";
const FILE_FILTER: &str = "debug";

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

pub fn init() -> anyhow::Result<()> {
    use std::fs::OpenOptions;

    use anyhow::Context;
    use env_logger::Env;

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

    let mut console = Builder::from_env(Env::new().default_filter_or(CONSOLE_FILTER));
    if ansi_error.is_some() {
        console.write_style(WriteStyle::Never);
    }
    let file = Builder::from_env(Env::new().default_filter_or(FILE_FILTER))
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
