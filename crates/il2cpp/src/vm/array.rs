use core::slice;

use dahlah_derive::Il2CppValue;

use crate::{
    api,
    vm::{object::Il2CppObject, r#type::Il2CppType, value::Il2CppValue},
};

use super::class::Il2CppClass;

#[derive(Debug, Clone, Copy, Il2CppValue)]
#[repr(transparent)]
pub struct Il2CppArray(pub usize);

impl Il2CppArray {
    #[inline]
    fn new_specific(array_class: Il2CppClass, size: usize) -> Il2CppArray {
        api::il2cpp_array_new_specific(array_class, size)
    }

    #[inline]
    pub fn new(array_class: Il2CppClass, len: usize) -> Self {
        Self::new_specific(array_class, len)
    }

    #[inline]
    pub fn class(&self) -> Il2CppClass {
        unsafe { Il2CppClass(*(self.0 as *const usize)) }
    }

    #[inline]
    pub fn monitor(&self) -> usize {
        unsafe { *((self.0 + 8) as *const usize) }
    }

    #[inline]
    pub fn bounds(&self) -> usize {
        unsafe { *((self.0 + 16) as *const usize) }
    }

    #[inline]
    pub fn len(&self) -> usize {
        assert!(self.0 != 0, "null managed array");
        let address = self.0.checked_add(24).expect("array pointer overflow");
        unsafe { utils::memory::read::<usize>(address).expect("unreadable array length") }
    }

    #[inline]
    pub fn first_item_ptr(&self) -> usize {
        self.0.checked_add(32).expect("array pointer overflow")
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn get<T>(&self, i: usize) -> &T {
        &self.as_slice::<T>()[i]
    }

    #[inline]
    pub fn get_mut<T>(&mut self, i: usize) -> &mut T {
        &mut self.as_mut_slice::<T>()[i]
    }

    #[inline]
    pub fn to_vec<T: Clone>(self) -> Vec<T> {
        self.as_slice::<T>().to_vec()
    }

    fn checked_len<T>(&self) -> usize {
        let length = self.len();
        let size = length
            .checked_mul(size_of::<T>())
            .expect("array size overflow");
        assert!(
            size <= isize::MAX as usize,
            "array exceeds Rust slice limit"
        );
        assert_eq!(
            self.first_item_ptr() % align_of::<T>(),
            0,
            "misaligned array elements"
        );
        assert!(
            size == 0 || utils::memory::is_readable(self.first_item_ptr(), size),
            "unreadable array elements"
        );
        length
    }

    pub fn as_slice<T>(&self) -> &[T] {
        let length = self.checked_len::<T>();
        unsafe { slice::from_raw_parts(self.first_item_ptr() as *const T, length) }
    }

    pub fn as_mut_slice<T>(&mut self) -> &mut [T] {
        let length = self.checked_len::<T>();
        unsafe {
            std::slice::from_raw_parts_mut(
                std::mem::transmute::<*const u8, *mut T>(self.first_item_ptr() as *const u8),
                length,
            )
        }
    }
}
