use std::{collections::BTreeMap, ffi::c_void};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    GetModuleFileNameW, GetModuleHandleExW,
};
use windows::Win32::System::Memory::{
    MEM_COMMIT, MEMORY_BASIC_INFORMATION, PAGE_EXECUTE, PAGE_EXECUTE_READ, PAGE_EXECUTE_READWRITE,
    PAGE_EXECUTE_WRITECOPY, PAGE_GUARD, PAGE_NOACCESS, VirtualQuery,
};
use windows::core::PCWSTR;

#[derive(Clone, Copy)]
struct Region {
    end: usize,
    readable: bool,
    executable: bool,
}

/// Operation-local protection cache for metadata that stays alive during a dump.
/// Reads still use SEH: a cached region is not a guarantee against concurrent unmapping.
#[derive(Default)]
pub struct RegionCache {
    regions: BTreeMap<usize, Region>,
    queries: usize,
}

impl RegionCache {
    fn region(&mut self, address: usize) -> Option<Region> {
        if address == 0 {
            return None;
        }
        if let Some((_, region)) = self.regions.range(..=address).next_back()
            && address < region.end
        {
            return Some(*region);
        }
        let mut info = MEMORY_BASIC_INFORMATION::default();
        self.queries += 1;
        if unsafe {
            VirtualQuery(
                Some(address as *const c_void),
                &mut info,
                size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        } == 0
        {
            return None;
        }
        let start = info.BaseAddress as usize;
        let end = start.checked_add(info.RegionSize)?;
        if start > address || end <= address {
            return None;
        }
        let accessible =
            info.State == MEM_COMMIT && info.Protect.0 & (PAGE_GUARD.0 | PAGE_NOACCESS.0) == 0;
        let region = Region {
            end,
            readable: accessible && info.Protect != PAGE_EXECUTE,
            executable: accessible
                && info.Protect.0
                    & (PAGE_EXECUTE.0
                        | PAGE_EXECUTE_READ.0
                        | PAGE_EXECUTE_READWRITE.0
                        | PAGE_EXECUTE_WRITECOPY.0)
                    != 0,
        };
        // Do not cache inaccessible regions: a runtime allocation may commit them later.
        if region.readable || region.executable {
            self.regions.insert(start, region);
        }
        Some(region)
    }

    fn is_readable(&mut self, address: usize, size: usize) -> bool {
        let Some(end) = address.checked_add(size) else {
            return false;
        };
        if address == 0 || size == 0 {
            return false;
        }
        let mut cursor = address;
        while cursor < end {
            let Some(region) = self.region(cursor) else {
                return false;
            };
            if !region.readable {
                return false;
            }
            cursor = region.end;
        }
        true
    }

    pub fn is_executable(&mut self, address: usize) -> bool {
        self.region(address).is_some_and(|region| region.executable)
    }

    pub fn query_count(&self) -> usize {
        self.queries
    }

    // Safety: the target must keep all bytes alive during the probe.
    pub unsafe fn probe(&mut self, address: usize, size: usize) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.is_readable(address, size),
            "unreadable range 0x{address:X}+0x{size:X}"
        );
        if let Err(error) = microseh::try_seh(|| {
            for offset in 0..size {
                // Volatile prevents a discarded probe result from eliminating the load.
                unsafe {
                    std::ptr::read_volatile((address + offset) as *const u8);
                }
            }
        }) {
            self.regions.clear();
            anyhow::bail!("cannot probe 0x{address:X}+0x{size:X}: {error:?}");
        }
        Ok(())
    }

    // Safety: the target must keep the value alive, just as for read().
    pub unsafe fn read<T: Copy>(&mut self, address: usize) -> anyhow::Result<T> {
        anyhow::ensure!(
            self.is_readable(address, size_of::<T>()),
            "unreadable address 0x{address:X}"
        );
        match microseh::try_seh(|| unsafe { std::ptr::read_unaligned(address as *const T) }) {
            Ok(value) => Ok(value),
            Err(error) => {
                self.regions.clear();
                Err(anyhow::anyhow!("cannot read 0x{address:X}: {error:?}"))
            }
        }
    }

    // Safety: the target must keep all UTF-16 code units alive during the copy.
    pub unsafe fn read_utf16(&mut self, address: usize, length: usize) -> anyhow::Result<String> {
        let size = length
            .checked_mul(size_of::<u16>())
            .ok_or_else(|| anyhow::anyhow!("UTF-16 byte length overflow"))?;
        anyhow::ensure!(address != 0, "null UTF-16 address");
        if size == 0 {
            return Ok(String::new());
        }
        anyhow::ensure!(
            self.is_readable(address, size),
            "unreadable UTF-16 range 0x{address:X}+0x{size:X}"
        );
        let mut units = Vec::new();
        units.try_reserve_exact(length)?;
        units.resize(length, 0u16);
        if let Err(error) = microseh::try_seh(|| unsafe {
            std::ptr::copy_nonoverlapping(
                address as *const u8,
                units.as_mut_ptr() as *mut u8,
                size,
            );
        }) {
            self.regions.clear();
            anyhow::bail!("cannot copy UTF-16 at 0x{address:X}: {error:?}");
        }
        Ok(String::from_utf16_lossy(&units))
    }
}

pub fn is_readable(address: usize, size: usize) -> bool {
    let Some(end) = address.checked_add(size) else {
        return false;
    };
    if address == 0 || size == 0 {
        return false;
    }
    let mut cursor = address;
    while cursor < end {
        let mut info = MEMORY_BASIC_INFORMATION::default();
        if unsafe {
            VirtualQuery(
                Some(cursor as *const c_void),
                &mut info,
                size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        } == 0
            || info.State != MEM_COMMIT
            || info.Protect.0 & (PAGE_GUARD.0 | PAGE_NOACCESS.0) != 0
            || info.Protect == PAGE_EXECUTE
        {
            return false;
        }
        let Some(next) = (info.BaseAddress as usize).checked_add(info.RegionSize) else {
            return false;
        };
        if next <= cursor {
            return false;
        }
        cursor = next;
    }
    true
}

pub fn is_executable(address: usize) -> bool {
    let mut info = MEMORY_BASIC_INFORMATION::default();
    let queried = unsafe {
        VirtualQuery(
            Some(address as *const c_void),
            &mut info,
            size_of::<MEMORY_BASIC_INFORMATION>(),
        )
    };
    queried != 0
        && info.State == MEM_COMMIT
        && info.Protect.0 & PAGE_GUARD.0 == 0
        && info.Protect.0
            & (PAGE_EXECUTE.0
                | PAGE_EXECUTE_READ.0
                | PAGE_EXECUTE_READWRITE.0
                | PAGE_EXECUTE_WRITECOPY.0)
            != 0
}

pub fn describe_address(address: usize) -> String {
    let mut module = HMODULE::default();
    if unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            PCWSTR(address as *const u16),
            &mut module,
        )
    }
    .is_err()
    {
        return format!("0x{address:X} (no loaded module)");
    }
    let base = module.0 as usize;
    let mut path = vec![0u16; 32768];
    let length = unsafe { GetModuleFileNameW(Some(module), &mut path) } as usize;
    let path = String::from_utf16_lossy(&path[..length]);
    let name = std::path::Path::new(&path)
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| "unknown module".into());
    match address.checked_sub(base) {
        Some(rva) => format!("{name}+0x{rva:X} (VA 0x{address:X}, base 0x{base:X})"),
        None => format!("0x{address:X} (module base 0x{base:X})"),
    }
}

// Safety:
// The target process must keep this memory alive while the value is read.
pub unsafe fn read<T: Copy>(address: usize) -> anyhow::Result<T> {
    anyhow::ensure!(
        is_readable(address, size_of::<T>()),
        "unreadable address 0x{address:X}"
    );
    microseh::try_seh(|| unsafe { std::ptr::read_unaligned(address as *const T) })
        .map_err(|error| anyhow::anyhow!("cannot read 0x{address:X}: {error:?}"))
}

// Safety: the target must keep the entire range alive during the copy.
pub unsafe fn read_bytes(address: usize, size: usize) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(
        is_readable(address, size),
        "unreadable range 0x{address:X}+0x{size:X}"
    );
    let mut bytes = vec![0; size];
    microseh::try_seh(|| unsafe {
        std::ptr::copy_nonoverlapping(address as *const u8, bytes.as_mut_ptr(), size);
    })
    .map_err(|error| anyhow::anyhow!("cannot copy 0x{address:X}+0x{size:X}: {error:?}"))?;
    Ok(bytes)
}
