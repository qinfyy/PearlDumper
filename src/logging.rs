use std::{
    fs::{File, OpenOptions},
    io::Write,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};
use windows::Win32::System::Threading::GetCurrentThreadId;

struct Logger(Mutex<File>);

impl log::Log for Logger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Debug
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        if let Ok(mut file) = self.0.lock() {
            let _ = writeln!(
                file,
                "[{}.{:03}] {} {} [tid={}]: {}",
                timestamp.as_secs(),
                timestamp.subsec_millis(),
                record.level(),
                record.target(),
                unsafe { GetCurrentThreadId() },
                record.args()
            );
            let _ = file.flush();
        }
        if record.level() <= log::Level::Info {
            let _ = writeln!(std::io::stdout(), "[{}] {}", record.level(), record.args());
        }
    }

    fn flush(&self) {
        if let Ok(mut file) = self.0.lock() {
            let _ = file.flush();
        }
    }
}

pub fn init() -> anyhow::Result<()> {
    dumper::ensure_output_directory()?;
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open("DUMP/dump.log")?;
    let logger = Box::leak(Box::new(Logger(Mutex::new(file))));
    log::set_logger(logger).map_err(|error| anyhow::anyhow!("cannot install logger: {error}"))?;
    log::set_max_level(log::LevelFilter::Debug);
    Ok(())
}
