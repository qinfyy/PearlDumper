use super::{object::Il2CppObject, r#type::Il2CppType, value::Il2CppValue};
use dahlah_derive::il2cpp_api;
use std::borrow::Cow;

#[derive(Clone, Copy, dahlah_derive::Il2CppValue)]
#[repr(transparent)]
pub struct Il2CppString(pub usize);

#[il2cpp_api("System.Runtime.InteropServices.Marshal")]
impl Il2CppString {
    #[method("PtrToStringAnsi", 24, native)]
    pub fn ptr_to_string(str: usize) -> anyhow::Result<Il2CppString>;

    #[method("PtrToStringUni", 25, native)]
    fn ptr_to_string_utf16(str: usize) -> anyhow::Result<Il2CppString>;
}

impl Il2CppString {
    #[inline]
    pub fn as_str(&self) -> Cow<'static, str> {
        unsafe {
            assert!(self.0 != 0, "null managed string");
            let str_length =
                utils::memory::read::<i32>(self.0 + 16).expect("invalid string length");
            assert!(str_length >= 0, "negative string length");
            if str_length == 0 {
                return Cow::Borrowed("");
            }
            let address = self.0.checked_add(20).expect("string pointer overflow");
            let size = (str_length as usize)
                .checked_mul(2)
                .expect("string length overflow");
            assert!(
                utils::memory::is_readable(address, size),
                "unreadable string data"
            );
            let slice = std::slice::from_raw_parts(address as *const u16, str_length as usize);
            String::from_utf16_lossy(slice).into()
        }
    }
}

impl From<&str> for Il2CppString {
    #[inline]
    fn from(value: &str) -> Self {
        assert!(
            !value.contains('\0'),
            "embedded NUL in managed string input"
        );
        let utf16 = value
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        Self::ptr_to_string_utf16(utf16.as_ptr() as usize).unwrap()
    }
}

impl From<String> for Il2CppString {
    #[inline]
    fn from(value: String) -> Self {
        Self::from(value.as_str())
    }
}
