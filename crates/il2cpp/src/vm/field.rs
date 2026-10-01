use crate::{api, vm::value::Il2CppValue};

#[derive(Debug, Clone, Copy)]
#[repr(transparent)]
pub struct Il2CppField(pub usize);

#[allow(unused)]
impl Il2CppField {
    #[inline]
    pub fn offset(&self) -> usize {
        api::il2cpp_field_get_offset(*self)
    }
}

impl Il2CppValue for Il2CppField {
    fn as_raw(&self) -> usize {
        std::ptr::from_ref(&self.0) as usize
    }
}
