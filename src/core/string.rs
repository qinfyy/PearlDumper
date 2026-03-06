use std::{borrow::Cow, ffi::CString};

use crate::{cs_class, cs_method};

#[derive(Debug, Copy, Clone)]
#[repr(transparent)]
pub struct SystemString(pub usize);

impl SystemString {
    cs_class!(&String::from("System.Runtime.InteropServices.Marshal"));

    cs_method!(ptr_to_string_ansi, "PtrToStringAnsi", &["System.IntPtr"], SystemString, (ptr: *const u8));

    #[inline(always)]
    pub fn as_str(&self) -> Cow<'static, str> {
        unsafe {
            let str_length = *(self.0.wrapping_add(16) as *const u32);
            let str_ptr = self.0.wrapping_add(20) as *const u16;
            let slice = std::slice::from_raw_parts(str_ptr, str_length as usize);
            String::from_utf16(slice).unwrap().into()
        }
    }
}

impl From<&str> for SystemString {
    #[inline(always)]
    fn from(s: &str) -> Self {
        let cs = CString::new(s).unwrap();
        Self::ptr_to_string_ansi(cs.as_c_str().to_bytes_with_nul().as_ptr())
            .expect("failed to allocate il2cpp string")
    }
}

impl From<String> for SystemString {
    #[inline(always)]
    fn from(value: String) -> Self {
        Self::from(value.as_str())
    }
}

impl From<usize> for SystemString {
    #[inline(always)]
    fn from(value: usize) -> Self {
        Self(value)
    }
}
