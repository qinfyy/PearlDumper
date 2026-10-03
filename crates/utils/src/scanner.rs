use std::{ffi::c_void, ops::Range, sync::OnceLock};

use patternscan::scan_first_match;
use windows::{
    Win32::System::{
        Diagnostics::Debug::ReadProcessMemory,
        LibraryLoader::GetModuleHandleA,
        Memory::{MEM_COMMIT, MEMORY_BASIC_INFORMATION, VirtualQuery},
        ProcessStatus::{GetModuleInformation, MODULEINFO},
        Threading::GetCurrentProcess,
    },
    core::{PCSTR, s},
};

struct ModuleSnapshot {
    bytes: Vec<u8>,
    executable: Vec<Range<usize>>,
}

impl ModuleSnapshot {
    fn capture(name: PCSTR) -> anyhow::Result<Self> {
        let module = unsafe { GetModuleHandleA(name) }?;
        let process = unsafe { GetCurrentProcess() };
        let mut info = MODULEINFO::default();
        unsafe {
            GetModuleInformation(process, module, &mut info, size_of::<MODULEINFO>() as u32)
        }?;
        let base = info.lpBaseOfDll as usize;
        let length = info.SizeOfImage as usize;
        anyhow::ensure!(
            base != 0 && length != 0 && length <= 2 * 1024 * 1024 * 1024,
            "invalid module range"
        );
        let end = base
            .checked_add(length)
            .ok_or_else(|| anyhow::anyhow!("module range overflow"))?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(length)?;
        bytes.resize(length, 0);
        let mut executable = Vec::new();
        let mut cursor = base;

        // Copy readable regions once. Never create a Rust slice spanning unmapped or guarded pages.
        while cursor < end {
            let mut region = MEMORY_BASIC_INFORMATION::default();
            let queried = unsafe {
                VirtualQuery(
                    Some(cursor as *const c_void),
                    &mut region,
                    size_of::<MEMORY_BASIC_INFORMATION>(),
                )
            };
            anyhow::ensure!(queried != 0, "VirtualQuery failed at 0x{cursor:X}");
            let next = (region.BaseAddress as usize)
                .checked_add(region.RegionSize)
                .ok_or_else(|| anyhow::anyhow!("memory region overflow"))?
                .min(end);
            anyhow::ensure!(next > cursor, "invalid memory region at 0x{cursor:X}");
            let range = cursor - base..next - base;
            if region.State == MEM_COMMIT && crate::memory::is_readable(cursor, next - cursor) {
                let mut copied = 0;
                unsafe {
                    ReadProcessMemory(
                        process,
                        cursor as *const c_void,
                        bytes[range.clone()].as_mut_ptr().cast(),
                        range.len(),
                        Some(&mut copied),
                    )
                }?;
                anyhow::ensure!(copied == range.len(), "partial module read at 0x{cursor:X}");
                if crate::memory::is_executable(cursor) {
                    executable.push(range);
                }
            }
            cursor = next;
        }
        anyhow::ensure!(
            !executable.is_empty(),
            "no readable executable regions in module"
        );
        log::debug!(
            "Snapshotted {} bytes in {} executable regions",
            length,
            executable.len()
        );
        Ok(Self { bytes, executable })
    }

    fn scan(&self, pattern: &str) -> Option<usize> {
        let mut found = None;
        for region in &self.executable {
            let mut offset = region.start;
            while offset < region.end {
                let local = scan_first_match(&mut &self.bytes[offset..region.end], pattern)
                    .unwrap_or_else(|error| panic!("invalid scan pattern {pattern}: {error}"));
                let Some(local) = local else { break };
                let address = offset + local;
                if found.replace(address).is_some() {
                    log::error!("Ambiguous executable pattern: {pattern}");
                    return None;
                }
                offset = address + 1;
            }
        }
        resolve_relative(&self.bytes, found?)
    }
}

fn cached_snapshot(
    storage: &'static OnceLock<Result<ModuleSnapshot, String>>,
    name: PCSTR,
) -> &'static ModuleSnapshot {
    storage
        .get_or_init(|| ModuleSnapshot::capture(name).map_err(|error| error.to_string()))
        .as_ref()
        .unwrap_or_else(|error| panic!("cannot snapshot module: {error}"))
}

fn game_assembly() -> &'static ModuleSnapshot {
    static SNAPSHOT: OnceLock<Result<ModuleSnapshot, String>> = OnceLock::new();
    cached_snapshot(&SNAPSHOT, s!("GameAssembly.dll"))
}

fn unity_player() -> &'static ModuleSnapshot {
    static SNAPSHOT: OnceLock<Result<ModuleSnapshot, String>> = OnceLock::new();
    cached_snapshot(&SNAPSHOT, s!("UnityPlayer.dll"))
}

pub fn game_assembly_slice() -> &'static [u8] {
    &game_assembly().bytes
}

pub fn game_assembly_code_sections() -> impl Iterator<Item = (usize, &'static [u8])> {
    let snapshot = game_assembly();
    snapshot
        .executable
        .iter()
        .map(move |range| (range.start, &snapshot.bytes[range.clone()]))
}

pub fn game_assembly_functions() -> anyhow::Result<&'static crate::pe::FunctionTable> {
    static TABLE: OnceLock<Result<crate::pe::FunctionTable, String>> = OnceLock::new();
    TABLE
        .get_or_init(|| {
            crate::pe::FunctionTable::from_mapped_pe(game_assembly_slice())
                .map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(|error| anyhow::anyhow!("invalid GameAssembly exception directory: {error}"))
}

pub fn game_assembly_function(rva: usize) -> anyhow::Result<&'static [u8]> {
    let range = game_assembly_functions()?
        .containing(rva)
        .ok_or_else(|| anyhow::anyhow!("no native function contains RVA 0x{rva:X}"))?;
    game_assembly_slice()
        .get(rva..range.end as usize)
        .ok_or_else(|| anyhow::anyhow!("native function outside GameAssembly: 0x{rva:X}"))
}

pub fn scan_ga_section(pattern: &str) -> Option<usize> {
    game_assembly().scan(pattern)
}

pub fn unity_player_slice() -> &'static [u8] {
    &unity_player().bytes
}

pub fn scan_unity_player_section(pattern: &str) -> Option<usize> {
    unity_player().scan(pattern)
}

fn xluau() -> &'static ModuleSnapshot {
    static SNAPSHOT: OnceLock<Result<ModuleSnapshot, String>> = OnceLock::new();
    cached_snapshot(&SNAPSHOT, s!("xluau.dll"))
}

pub fn xluau_bytes() -> &'static [u8] {
    &xluau().bytes
}

pub fn scan_by_pattern(pattern: &str) -> Option<usize> {
    xluau().scan(pattern)
}

fn resolve_relative(bytes: &[u8], address: usize) -> Option<usize> {
    let instruction = bytes.get(address..)?;
    let (length, displacement_offset) = match *instruction.first()? {
        0xE8 | 0xE9 => (5, 1),
        0x48 if matches!(instruction.get(1), Some(0x8B | 0x8D))
            && instruction.get(2).is_some_and(|modrm| modrm & 0xC7 == 0x05) =>
        {
            (7, 3)
        }
        _ => return Some(address),
    };
    let displacement = i32::from_le_bytes(
        instruction
            .get(displacement_offset..length)?
            .try_into()
            .ok()?,
    );
    let target = address
        .checked_add(length)?
        .checked_add_signed(displacement as isize)?;
    (target < bytes.len()).then_some(target)
}
