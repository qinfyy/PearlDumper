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
        resolve_relative(&self.bytes, self.find_unique(pattern)?)
    }

    fn find_unique(&self, pattern: &str) -> Option<usize> {
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
        found
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
    game_assembly_functions()?.function_bytes(game_assembly_slice(), rva)
}

pub fn game_assembly_candidate(rva: usize) -> anyhow::Result<Option<&'static [u8]>> {
    game_assembly_functions()?.candidate_bytes(game_assembly_slice(), rva)
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

pub fn unity_api_table_rva() -> anyhow::Result<usize> {
    let found = find_unity_api_table(&unity_player().bytes)?;
    log::debug!(
        "Unity init call +0x{:X}: MOV loads API table +0x{:X}; LEA passes domain string +0x{:X}",
        found.reference_rva,
        found.table_rva,
        found.domain_name_rva
    );
    Ok(found.table_rva)
}

#[derive(Debug)]
pub struct UnityApiTable {
    pub reference_rva: usize,
    pub table_rva: usize,
    pub domain_name_rva: usize,
}

// Shared with offline replay: MOV loads table[0], LEA supplies the domain name.
pub fn find_unity_api_table(image: &[u8]) -> anyhow::Result<UnityApiTable> {
    let sections = crate::pe::mapped_sections(image)?;
    let functions = crate::pe::FunctionTable::from_mapped_pe(image)?;
    let mut accepted = Vec::new();
    let mut rejected = Vec::new();
    for section in sections
        .iter()
        .filter(|s| s.characteristics & 0x2000_0000 != 0)
    {
        let mut cursor = section.range.start;
        while cursor < section.range.end {
            let Some(local) = scan_first_match(
                &mut &image[cursor..section.range.end],
                "48 8B 05 ? ? ? ? 48 8D 0D ? ? ? ? FF D0",
            )?
            else {
                break;
            };
            let reference = cursor + local;
            cursor = reference + 1;
            let candidate = (|| -> anyhow::Result<UnityApiTable> {
                let function = functions
                    .containing(reference)
                    .ok_or_else(|| anyhow::anyhow!("call site has no function boundary"))?;
                anyhow::ensure!(
                    reference + 16 <= function.end as usize,
                    "call crosses function boundary"
                );
                let table_rva = resolve_relative(image, reference)
                    .ok_or_else(|| anyhow::anyhow!("invalid MOV target"))?;
                validate_unity_api_storage(image, table_rva)?;
                let domain_name_rva = resolve_relative(image, reference + 7)
                    .ok_or_else(|| anyhow::anyhow!("invalid LEA argument"))?;
                let name = b"IL2CPP Root Domain\0";
                anyhow::ensure!(
                    image.get(domain_name_rva..domain_name_rva + name.len()) == Some(name),
                    "call argument is not the IL2CPP root domain name"
                );
                anyhow::ensure!(
                    sections.iter().any(|s| s.range.contains(&domain_name_rva)
                        && domain_name_rva + name.len() <= s.range.end
                        && s.characteristics & 0xE000_0000 == 0x4000_0000),
                    "domain name is not in read-only data"
                );
                Ok(UnityApiTable {
                    reference_rva: reference,
                    table_rva,
                    domain_name_rva,
                })
            })();
            match candidate {
                Ok(found) => accepted.push(found),
                Err(error) => rejected.push(format!("0x{reference:X}: {error:#}")),
            }
        }
    }
    anyhow::ensure!(
        accepted.len() == 1,
        "expected one validated Unity API table, found {}; rejected: {}",
        accepted.len(),
        rejected.join("; ")
    );
    Ok(accepted.remove(0))
}

pub fn validate_unity_api_storage(image: &[u8], rva: usize) -> anyhow::Result<()> {
    let end = rva
        .checked_add(171 * 8)
        .ok_or_else(|| anyhow::anyhow!("API table overflow"))?;
    anyhow::ensure!(
        rva.is_multiple_of(8) && end <= image.len(),
        "API table outside image or unaligned"
    );
    anyhow::ensure!(
        crate::pe::mapped_sections(image)?
            .iter()
            .any(|s| rva >= s.range.start
                && end <= s.range.end
                && s.characteristics & 0xE000_0000 == 0xC000_0000),
        "API table +0x{rva:X} is not fully in readable/writable, non-executable storage"
    );
    Ok(())
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
