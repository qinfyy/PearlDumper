use std::ffi::c_void;
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
