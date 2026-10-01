use std::ffi::c_void;
use windows::Win32::System::Memory::{
    MEM_COMMIT, MEMORY_BASIC_INFORMATION, PAGE_EXECUTE, PAGE_EXECUTE_READ, PAGE_EXECUTE_READWRITE,
    PAGE_EXECUTE_WRITECOPY, PAGE_GUARD, PAGE_NOACCESS, VirtualQuery,
};

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

/// # Safety
/// The target process must keep this memory alive while the value is read.
pub unsafe fn read<T: Copy>(address: usize) -> anyhow::Result<T> {
    anyhow::ensure!(
        is_readable(address, size_of::<T>()),
        "unreadable address 0x{address:X}"
    );
    microseh::try_seh(|| unsafe { std::ptr::read_unaligned(address as *const T) })
        .map_err(|error| anyhow::anyhow!("cannot read 0x{address:X}: {error:?}"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn rejects_invalid_ranges() {
        assert!(!super::is_readable(0, 8));
        assert!(!super::is_readable(usize::MAX - 3, 8));
        assert!(!super::is_readable(1, 8));
    }

    #[test]
    fn reads_owned_memory() {
        let value = 1234u64;
        assert_eq!(
            unsafe { super::read::<u64>(&value as *const _ as usize) }.unwrap(),
            value
        );
    }
}
