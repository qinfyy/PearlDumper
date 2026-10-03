use std::{borrow::Cow, ffi::CStr};

pub mod interceptor;
pub mod memory;
pub mod pe;
pub mod scanner;

pub use interceptor::Interceptor;
pub use scanner::{game_assembly_slice, scan_ga_section, scan_unity_player_section};

// Safety:
// `ptr` must point to a valid, null-terminated string for the returned lifetime.
pub unsafe fn cstr_to_str(ptr: *const i8) -> Cow<'static, str> {
    if ptr.is_null() {
        return Cow::Borrowed("");
    }
    unsafe { CStr::from_ptr(ptr).to_string_lossy() }
}
