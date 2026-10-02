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

#[cfg(not(test))]
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

#[cfg(test)]
mod tests {
    use std::{
        io::{self, Write},
        sync::{Arc, Mutex},
    };

    use log::{Level, Log, Record};

    use super::*;

    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);

    impl Capture {
        fn text(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }

    impl Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn output(filter: &str, style: WriteStyle) -> (env_logger::Logger, Capture) {
        let capture = Capture::default();
        let logger = Builder::new()
            .parse_filters(filter)
            .write_style(style)
            .format_timestamp(None)
            .target(Target::Pipe(Box::new(capture.clone())))
            .build();
        (logger, capture)
    }

    fn logger(console_filter: &str, file_filter: &str) -> (Logger, Capture, Capture) {
        let (console, console_capture) = output(console_filter, WriteStyle::Never);
        let (file, file_capture) = output(file_filter, WriteStyle::Never);
        (Logger { console, file }, console_capture, file_capture)
    }

    fn emit(logger: &Logger, level: Level, target: &str, message: &str) {
        logger.log(
            &Record::builder()
                .level(level)
                .target(target)
                .args(format_args!("{message}"))
                .build(),
        );
    }

    #[test]
    fn defaults_keep_debug_in_file_and_info_in_console() {
        let (logger, console, file) = logger(CONSOLE_FILTER, FILE_FILTER);
        emit(&logger, Level::Info, "il2cpp", "runtime ready");
        emit(&logger, Level::Debug, "il2cpp", "method address");
        emit(&logger, Level::Trace, "il2cpp", "trace detail");
        assert!(console.text().contains("runtime ready"));
        assert!(!console.text().contains("method address"));
        assert!(file.text().contains("runtime ready"));
        assert!(file.text().contains("method address"));
        assert!(!file.text().contains("trace detail"));
    }

    #[test]
    fn module_filters_apply_to_both_outputs() {
        let (logger, console, file) = logger("warn,il2cpp=trace", "warn,il2cpp=trace");
        emit(&logger, Level::Info, "other", "filtered message");
        emit(&logger, Level::Trace, "il2cpp::api", "slot details");
        for output in [console.text(), file.text()] {
            assert!(!output.contains("filtered message"));
            assert!(output.contains("slot details"));
        }
    }

    #[test]
    fn regex_filters_are_not_bypassed() {
        let (logger, console, file) = logger("info/keep", "info/keep");
        emit(&logger, Level::Info, "il2cpp", "discard message");
        emit(&logger, Level::Info, "il2cpp", "keep message");
        for output in [console.text(), file.text()] {
            assert!(!output.contains("discard message"));
            assert!(output.contains("keep message"));
        }
    }

    #[test]
    fn console_color_does_not_leak_into_file() {
        let (console, console_capture) = output("info", WriteStyle::Always);
        let (file, file_capture) = output("debug", WriteStyle::Never);
        let logger = Logger { console, file };
        for _ in 0..2 {
            emit(&logger, Level::Info, "il2cpp", "colored console");
        }
        assert!(console_capture.text().contains("\x1b["));
        assert!(!file_capture.text().contains("\x1b["));
        assert_eq!(file_capture.text().matches("colored console").count(), 2);
    }

    #[test]
    fn tracing_and_log_reach_each_output_once() {
        let filter = "off,castorice_logging_test=trace";
        let (logger, console, file) = logger(filter, filter);
        install(logger).unwrap();
        tracing::info!(target: "castorice_logging_test", "from tracing");
        log::info!(target: "castorice_logging_test", "from log");
        tracing::trace!(target: "castorice_logging_test", "trace bridge");
        for output in [console.text(), file.text()] {
            assert_eq!(output.matches("from tracing").count(), 1);
            assert_eq!(output.matches("from log").count(), 1);
            assert_eq!(output.matches("trace bridge").count(), 1);
        }
    }
}
