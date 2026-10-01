use crate::runtime;
use crate::script_v2::{scanner::Scanner, writer::Writer};

mod constants;
mod context;
mod models;
mod naming_utils;
mod scanner;
pub mod struct_init;
mod type_analyzer;
mod type_registry;
mod writer;

pub fn dump() -> anyhow::Result<()> {
    let handle = std::thread::Builder::new()
        .name("script-v2-dumper".into())
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            crate::guarded(|| {
                runtime::attach_current_thread_to_il2cpp();
                let mut scanner = Scanner::default();
                scanner.init();

                let mut writer = Writer::new(scanner);
                writer.save_string_literals();
                writer.save_script();
                writer.save_struct();
                Ok(())
            })
        })?;
    handle
        .join()
        .map_err(|error| anyhow::anyhow!("script worker panic: {}", crate::panic_message(error)))?
}
