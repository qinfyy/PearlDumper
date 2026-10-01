use crate::api;

use super::{class::Il2CppClass, r#type::Il2CppType, value::Il2CppValue};
use dahlah_derive::Il2CppValue;

#[derive(Clone, Copy, Il2CppValue)]
#[repr(transparent)]
pub struct Il2CppObject(pub usize);

impl Il2CppObject {
    pub const NULL: Il2CppObject = Il2CppObject(0);

    #[inline]
    pub fn new_object(class: Il2CppClass) -> Il2CppObject {
        api::il2cpp_object_new(class)
    }

    #[inline]
    pub fn object_box<T: Copy>(class: Il2CppClass, value: *const T) -> Il2CppObject {
        api::il2cpp_value_box(class, value)
    }

    #[inline]
    pub fn box_value<T: Copy>(class: Il2CppClass, value: *const T) -> Self {
        Self::object_box(class, value)
    }

    #[inline]
    pub fn get_class(&self) -> Il2CppClass {
        unsafe {
            Il2CppClass(utils::memory::read::<usize>(self.0).expect("unreadable object class"))
        }
    }

    #[inline]
    pub fn unbox<T: Copy>(&self) -> T {
        let address = self
            .0
            .checked_add(16)
            .expect("boxed object address overflow");
        unsafe { utils::memory::read::<T>(address).expect("unreadable boxed value") }
    }
}
