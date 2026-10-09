use anyhow::{Context, ensure};
use std::{path::PathBuf, sync::OnceLock};
use windows::Win32::System::LibraryLoader::{GetModuleFileNameW, GetModuleHandleW};
use windows::core::{HSTRING, PCWSTR};

pub struct ModuleInfo {
    pub image_size: u32,
    pub timestamp: u32,
}
pub struct LoadedClient {
    pub assembly: ModuleInfo,
    pub api_table_rva: usize,
}

pub fn current() -> anyhow::Result<&'static LoadedClient> {
    static CLIENT: OnceLock<Result<LoadedClient, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| discover().map_err(|error| format!("{error:#}")))
        .as_ref()
        .map_err(|error| anyhow::anyhow!("client discovery failed: {error}"))
}

fn discover() -> anyhow::Result<LoadedClient> {
    let (_, ga) = loaded_module_path("GameAssembly.dll")?;
    let (_, up) = loaded_module_path("UnityPlayer.dll")?;
    let assembly = module_info(ga)?;
    let unity = module_info(up)?;
    let api_table_rva = utils::scanner::unity_api_table_rva()?;
    ensure!(
        api_table_rva.is_multiple_of(8)
            && api_table_rva
                .checked_add(171 * 8)
                .is_some_and(|end| end <= unity.image_size as usize),
        "API table is outside UnityPlayer"
    );
    log::info!(
        "Dynamic client: GameAssembly timestamp {:08X}, image 0x{:X}; Unity API table +0x{api_table_rva:X}",
        assembly.timestamp,
        assembly.image_size
    );
    Ok(LoadedClient {
        assembly,
        api_table_rva,
    })
}

pub fn loaded_module_path(name: &str) -> anyhow::Result<(PathBuf, usize)> {
    let wide = HSTRING::from(name);
    let module = unsafe { GetModuleHandleW(PCWSTR(wide.as_ptr())) }?;
    let mut buffer = vec![0u16; 32768];
    let length = unsafe { GetModuleFileNameW(Some(module), &mut buffer) } as usize;
    ensure!(
        length > 0 && length < buffer.len(),
        "cannot resolve loaded module {name}"
    );
    Ok((
        PathBuf::from(String::from_utf16(&buffer[..length])?),
        module.0 as usize,
    ))
}

fn module_info(base: usize) -> anyhow::Result<ModuleInfo> {
    unsafe {
        ensure!(
            utils::memory::read::<u16>(base)? == 0x5A4D,
            "invalid DOS header"
        );
        let offset = utils::memory::read::<u32>(base + 0x3C)? as usize;
        ensure!(offset <= 1024 * 1024, "invalid PE offset");
        let nt = base.checked_add(offset).context("PE address overflow")?;
        ensure!(
            utils::memory::read::<u32>(nt)? == 0x4550
                && utils::memory::read::<u16>(nt + 4)? == 0x8664
                && utils::memory::read::<u16>(nt + 24)? == 0x20B,
            "expected x64 PE32+ module"
        );
        let image_size = utils::memory::read::<u32>(nt + 80)?;
        ensure!(
            image_size > 4096 && image_size <= 2 * 1024 * 1024 * 1024,
            "invalid image size"
        );
        Ok(ModuleInfo {
            image_size,
            timestamp: utils::memory::read::<u32>(nt + 8)?,
        })
    }
}
