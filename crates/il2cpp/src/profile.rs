use anyhow::Context;
use serde::Deserialize;
use std::path::PathBuf;
use windows::Win32::System::LibraryLoader::{GetModuleFileNameW, GetModuleHandleW};
use windows::core::{HSTRING, PCWSTR};

#[derive(Debug, Deserialize)]
pub struct BinaryFingerprint {
    pub name: String,
    pub file_size: u64,
    pub sha256: String,
    pub timestamp: u32,
    pub image_size: u32,
    pub machine: u16,
}

#[derive(Debug, Deserialize)]
pub struct ClientProfile {
    pub version: String,
    pub unity: BinaryFingerprint,
    pub assembly: BinaryFingerprint,
    pub api_table_rva: usize,
}

pub fn supported() -> anyhow::Result<ClientProfile> {
    Ok(serde_json::from_str(include_str!(
        "../../../profiles/4.6.51.json"
    ))?)
}

pub fn validate_loaded_client() -> anyhow::Result<ClientProfile> {
    let profile = supported()?;
    log::info!(
        "Client {}: checking loaded PE headers only; file hashing is disabled",
        profile.version
    );
    for binary in [&profile.unity, &profile.assembly] {
        let (path, base) = loaded_module_path(&binary.name)
            .with_context(|| format!("locating loaded {}", binary.name))?;
        log::info!(
            "Checking {} PE header at 0x{base:X} ({})",
            binary.name,
            path.display()
        );
        validate_loaded_header(base, binary)
            .with_context(|| format!("checking {} loaded PE header", binary.name))?;
    }
    log::info!("Loaded PE headers match client {}", profile.version);
    Ok(profile)
}

fn loaded_module_path(name: &str) -> anyhow::Result<(PathBuf, usize)> {
    let wide = HSTRING::from(name);
    let module = unsafe { GetModuleHandleW(PCWSTR(wide.as_ptr())) }?;
    let mut buffer = vec![0u16; 32768];
    let length = unsafe { GetModuleFileNameW(Some(module), &mut buffer) } as usize;
    anyhow::ensure!(
        length > 0 && length < buffer.len(),
        "cannot resolve loaded module {name}"
    );
    Ok((
        PathBuf::from(String::from_utf16(&buffer[..length])?),
        module.0 as usize,
    ))
}

fn validate_loaded_header(base: usize, expected: &BinaryFingerprint) -> anyhow::Result<()> {
    unsafe {
        anyhow::ensure!(
            utils::memory::read::<u16>(base)? == 0x5A4D,
            "{}: invalid DOS header",
            expected.name
        );
        let offset = utils::memory::read::<u32>(base + 0x3C)? as usize;
        anyhow::ensure!(
            offset <= 1024 * 1024,
            "{}: invalid PE offset",
            expected.name
        );
        let nt = base
            .checked_add(offset)
            .ok_or_else(|| anyhow::anyhow!("PE address overflow"))?;
        anyhow::ensure!(
            utils::memory::read::<u32>(nt)? == 0x4550,
            "{}: invalid PE signature",
            expected.name
        );
        anyhow::ensure!(
            utils::memory::read::<u16>(nt + 4)? == expected.machine
                && utils::memory::read::<u32>(nt + 8)? == expected.timestamp
                && utils::memory::read::<u16>(nt + 24)? == 0x20B
                && utils::memory::read::<u32>(nt + 80)? == expected.image_size,
            "{}: loaded PE header does not match the supported client",
            expected.name
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn profile_matches_target() {
        let profile = super::supported().unwrap();
        assert_eq!(profile.version, "4.6.51");
        assert_eq!(profile.api_table_rva, 0x20AA978);
        assert_eq!(profile.assembly.file_size, 560853280);
        assert_eq!(profile.unity.machine, 0x8664);
        assert_eq!(profile.assembly.sha256.len(), 64);
    }

    #[test]
    fn loaded_header_mismatch_is_rejected() {
        let expected = super::supported().unwrap().assembly;
        let mut header = [0u8; 256];
        header[..2].copy_from_slice(&0x5A4Du16.to_le_bytes());
        header[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        header[0x80..0x84].copy_from_slice(&0x4550u32.to_le_bytes());
        header[0x84..0x86].copy_from_slice(&expected.machine.to_le_bytes());
        header[0x88..0x8C].copy_from_slice(&expected.timestamp.to_le_bytes());
        header[0x98..0x9A].copy_from_slice(&0x20Bu16.to_le_bytes());
        header[0xD0..0xD4].copy_from_slice(&expected.image_size.to_le_bytes());
        assert!(super::validate_loaded_header(header.as_ptr() as usize, &expected).is_ok());
        header[0x88] ^= 1;
        assert!(super::validate_loaded_header(header.as_ptr() as usize, &expected).is_err());
    }
}
