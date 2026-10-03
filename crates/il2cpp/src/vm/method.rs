use crate::api;

use super::{
    class::Il2CppClass, exception::Il2CppException, object::Il2CppObject, value::Il2CppValue,
};
use std::{borrow::Cow, sync::OnceLock};

#[derive(Debug, Clone, Copy)]
#[repr(transparent)]
pub struct Il2CppMethod(pub usize);

impl Il2CppMethod {
    fn get_class_va_offset_pair(&self) -> (usize, usize) {
        unsafe {
            static CACHE: OnceLock<(usize, usize)> = OnceLock::new();
            let &(class, va) = CACHE.get_or_init(|| {
                let class = self.class().0;
                let first = utils::memory::read::<usize>(self.0).expect("invalid MethodInfo");
                let second = utils::memory::read::<usize>(self.0 + 8).expect("invalid MethodInfo");
                let possibilities = if class != 0 && first == class {
                    (0, 8)
                } else if class != 0 && second == class {
                    (8, 0)
                } else {
                    panic!("unsupported MethodInfo layout: class=0x{class:X}, first=0x{first:X}, second=0x{second:X}");
                };
                log::debug!("MethodInfo class offset {}, pointer offset {}", possibilities.0, possibilities.1);

                possibilities
            });

            (class, va)
        }
    }

    #[inline]
    pub fn class(&self) -> Il2CppClass {
        api::il2cpp_method_get_class(*self)
    }

    #[inline]
    pub fn va(&self) -> usize {
        assert!(self.0 != 0, "cannot read a null native MethodInfo");
        let (_, va) = self.get_class_va_offset_pair();
        // Handles come from IL2CPP metadata and remain alive for the dump. The
        // layout is checked once; the enclosing dump stage owns SEH protection.
        unsafe { std::ptr::read_unaligned((self.0 + va) as *const usize) }
    }

    pub fn checked_va(&self) -> anyhow::Result<usize> {
        let va = self.va();
        anyhow::ensure!(
            utils::memory::is_executable(va),
            "method pointer is not executable: 0x{va:X}"
        );
        Ok(va)
    }

    pub fn checked_rva(&self) -> anyhow::Result<usize> {
        let va = self.checked_va()?;
        let rva = va.checked_sub(*crate::GA_BASE).ok_or_else(|| {
            anyhow::anyhow!("method 0x{:X} is outside GameAssembly: 0x{va:X}", self.0)
        })?;
        anyhow::ensure!(
            rva < crate::profile::supported()?.assembly.image_size as usize,
            "method RVA outside GameAssembly: 0x{rva:X}"
        );
        Ok(rva)
    }

    #[inline]
    pub fn rva(&self) -> usize {
        let va = self.va();
        if va == 0 {
            return 0;
        }
        va.checked_sub(*crate::GA_BASE)
            .expect("method is outside GameAssembly")
    }

    #[inline]
    pub fn get_name(&self) -> Cow<'static, str> {
        unsafe { utils::cstr_to_str(api::il2cpp_method_get_name(*self)) }
    }

    pub fn invoke<T: From<usize>>(
        &self,
        instance: Il2CppObject,
        args: &[&dyn Il2CppValue],
    ) -> Result<T, Il2CppException> {
        let args = args.iter().map(|arg| arg.as_raw()).collect::<Vec<_>>();

        let mut exception = 0;
        let ret = api::il2cpp_runtime_invoke(*self, instance, args.as_ptr(), &mut exception);

        (exception == 0)
            .then_some(T::from(ret))
            .ok_or(Il2CppException(exception))
    }

    // Value-type arguments point to their payload; reference arguments use the object pointer.
    pub fn invoke2<T: From<usize>>(
        &self,
        instance: Il2CppObject,
        args: Vec<usize>,
    ) -> Result<T, Il2CppException> {
        let mut exception = 0;
        let ret = api::il2cpp_runtime_invoke(*self, instance, args.as_ptr(), &mut exception);

        (exception == 0)
            .then_some(T::from(ret))
            .ok_or(Il2CppException(exception))
    }
}

impl Il2CppMethod {
    pub fn signature(&self, index_in_class: usize) -> String {
        let name: Cow<'static, str> = self.get_name();
        format!("{}{}", name.as_ref(), index_in_class)
    }
}

impl Il2CppValue for Il2CppMethod {
    fn as_raw(&self) -> usize {
        std::ptr::from_ref(&self.0) as usize
    }
}
