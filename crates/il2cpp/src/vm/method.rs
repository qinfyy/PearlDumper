use crate::api;

use super::{
    class::Il2CppClass, exception::Il2CppException, object::Il2CppObject, r#type::Il2CppType,
    value::Il2CppValue,
};
use std::{borrow::Cow, sync::OnceLock};

#[derive(Debug, Clone, Copy)]
#[repr(transparent)]
pub struct Il2CppMethod(pub usize);

impl Il2CppMethod {
    fn get_class_va_offset_pair(&self) -> (usize, usize) {
        self.try_get_class_va_offset_pair()
            .expect("unsupported MethodInfo layout")
    }

    fn try_get_class_va_offset_pair(&self) -> anyhow::Result<(usize, usize)> {
        anyhow::ensure!(self.0 != 0, "cannot inspect a null native MethodInfo");
        unsafe {
            static CACHE: OnceLock<(usize, usize)> = OnceLock::new();
            if let Some(pair) = CACHE.get() {
                return Ok(*pair);
            }
            let class = self.class().0;
            let first = utils::memory::read::<usize>(self.0)?;
            let second = utils::memory::read::<usize>(self.0 + 8)?;
            let pair = if class != 0 && first == class {
                (0, 8)
            } else if class != 0 && second == class {
                (8, 0)
            } else {
                anyhow::bail!(
                    "unsupported MethodInfo layout: class=0x{class:X}, first=0x{first:X}, second=0x{second:X}"
                );
            };
            log::debug!(
                "MethodInfo class offset {}, pointer offset {}",
                pair.0,
                pair.1
            );
            let _ = CACHE.set(pair);
            if let Some(cached) = CACHE.get() {
                Ok(*cached)
            } else {
                Ok(pair)
            }
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

    pub fn try_va(&self) -> anyhow::Result<usize> {
        anyhow::ensure!(self.0 != 0, "cannot read a null native MethodInfo");
        let (_, va) = self.try_get_class_va_offset_pair()?;
        unsafe { utils::memory::read::<usize>(self.0 + va) }
    }

    pub fn try_va_cached(&self, memory: &mut utils::memory::RegionCache) -> anyhow::Result<usize> {
        anyhow::ensure!(self.0 != 0, "cannot read a null native MethodInfo");
        let (_, offset) = self.try_get_class_va_offset_pair()?;
        let address = self
            .0
            .checked_add(offset)
            .ok_or_else(|| anyhow::anyhow!("MethodInfo address overflow"))?;
        unsafe { memory.read::<usize>(address) }
    }

    pub fn checked_va(&self) -> anyhow::Result<usize> {
        let va = self.try_va()?;
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
            rva < crate::profile::current()?.assembly.image_size as usize,
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

    pub fn get_return_type(&self) -> anyhow::Result<Il2CppType> {
        anyhow::ensure!(self.0 != 0, "null native MethodInfo has no return type");
        let ty = api::il2cpp_method_get_return_type(*self);
        anyhow::ensure!(
            !ty.is_null(),
            "null native return type: method=0x{:X}, name={:?}",
            self.0,
            self.get_name()
        );
        Ok(ty)
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

// Decode the complete leaf API: TEST byte [RCX+flags], Static; SETE AL; RET.
// The offset belongs to the loaded runtime, not a version-specific MethodInfo struct.
pub fn method_flags_offset_from_instance(code: &[u8]) -> anyhow::Result<usize> {
    let (offset, tail) = match code {
        [0xf6, 0x41, offset, tail @ ..] => (usize::from(*offset), tail),
        [0xf6, 0x81, a, b, c, d, tail @ ..] => {
            let offset = i32::from_le_bytes([*a, *b, *c, *d]);
            anyhow::ensure!(offset > 0, "invalid method flags displacement");
            (offset as usize, tail)
        }
        _ => {
            anyhow::bail!("unsupported native method instance API; refusing to guess method flags")
        }
    };
    anyhow::ensure!(
        (1..=4096).contains(&offset) && tail.starts_with(&[0x10, 0x0f, 0x94, 0xc0, 0xc3]),
        "unsupported native method instance API tail or displacement"
    );
    Ok(offset)
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
