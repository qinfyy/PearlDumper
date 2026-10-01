extern crate self as il2cpp;
use crate::vm::{class::Il2CppClass, method::Il2CppMethod};
use indexmap::IndexMap;
use std::{
    borrow::Cow,
    sync::{LazyLock, OnceLock},
};
use windows::{Win32::System::LibraryLoader::GetModuleHandleW, core::w};
pub mod api;
pub mod api_table;
pub mod method_dump;
pub mod profile;
pub mod vm;

mod api_init;

pub use api_init::init_il2cpp;

pub static GA_BASE: LazyLock<usize> = LazyLock::new(|| unsafe {
    GetModuleHandleW(w!("GameAssembly"))
        .expect("GameAssembly not loaded yet")
        .0 as usize
});
pub static UP_BASE: LazyLock<usize> = LazyLock::new(|| unsafe {
    GetModuleHandleW(w!("UnityPlayer"))
        .expect("UnityPlayer not loaded yet")
        .0 as usize
});
pub static API_BASE_PTR: LazyLock<usize> = LazyLock::new(|| {
    profile::supported()
        .expect("invalid embedded client profile")
        .api_table_rva
});
pub static mut MAX_TYPEDEFINDEX: u32 = 0;
pub static mut RPG_NETWORK_PROTO_START: u32 = 0;
pub static mut RPG_NETWORK_PROTO_END: u32 = 0;
pub static mut ASSEMBLY_CSHARP_START: u32 = 0;
pub static FUNCTIONS_TABLE: OnceLock<IndexMap<String, Il2CppMethod>> = OnceLock::new();
pub static FUNCTIONS_TABLE_REFLECTION: OnceLock<IndexMap<String, Il2CppMethod>> = OnceLock::new();
pub static CLASS_TABLE: OnceLock<IndexMap<Cow<'static, str>, Il2CppClass>> = OnceLock::new();
pub static CLASS_TABLE_VEC: OnceLock<Vec<Il2CppClass>> = OnceLock::new();

#[macro_export]
macro_rules! native_method {
    ($vis:vis $func_name:ident, $ret_type:ty, ($($arg_name:ident: $arg_type:ty),*), $fn_sig:literal, self) => {
        #[allow(clippy::wrong_self_convention)]
        #[inline]
        $vis fn $func_name(&self, $($arg_name: $arg_type),*) -> anyhow::Result<$ret_type> {
            #[allow(clippy::macro_metavars_in_unsafe)]
            unsafe {
                let func: extern "C" fn(usize, $($arg_type),*) -> $ret_type = std::mem::transmute($crate::get_native_method($fn_sig).unwrap().va());
                let result = microseh::try_seh(|| func(self.0, $($arg_name),*));
                result.map_err(|e| anyhow::anyhow!("failed to invoke {}. {:?}", stringify!($func_name), e))
            }
        }
    };

    ($vis:vis $func_name:ident, $ret_type:ty, ($($arg_name:ident: $arg_type:ty),*), $fn_sig:literal) => {
        #[inline]
        $vis fn $func_name($($arg_name: $arg_type),*) -> anyhow::Result<$ret_type> {
            #[allow(clippy::macro_metavars_in_unsafe)]
            unsafe {
                let func: extern "C" fn($($arg_type),*) -> $ret_type = std::mem::transmute($crate::get_native_method($fn_sig).unwrap().va());
                let result = microseh::try_seh(|| func($($arg_name),*));
                result.map_err(|e| anyhow::anyhow!("failed to invoke {}. {:?}", stringify!($func_name), e))
            }
        }
    };

    // Generic
    (
        $vis:vis $func_name:ident,
        $ret_type:ty,
        < $($generic:ident : $bound:tt),+ > ($($arg_name:ident: $arg_type:ty),*),
        $fn_sig:literal,
        self
    ) => {
        #[inline]
        $vis  $func_name < $($generic : $bound),+ > (&self, $($arg_name: $arg_type),*) -> anyhow::Result<$ret_type> {
            let func: extern "fastcall" fn(usize, $($arg_type),*) -> $ret_type = std::mem::transmute($crate::get_native_method($fn_sig).unwrap().va());
            let result = microseh::try_seh(|| func(self.0, $($arg_name),*));
            Ok(result.map_err(|e| anyhow::anyhow!("failed to invoke {}. {:?}", stringify!($func_name), e))?)
        }
    };

    (runtime $vis:vis $func_name:ident, $ret_type:ty, ($($arg_name:ident: $arg_type:ty),*), $fn_sig:literal, self) => {
        #[allow(clippy::wrong_self_convention)]
        #[inline]
        $vis fn $func_name(&self, $($arg_name: $arg_type),*) -> anyhow::Result<$ret_type> {
            #[allow(clippy::macro_metavars_in_unsafe)]
            unsafe {
                let result = $crate::get_native_method($fn_sig).unwrap().invoke(Il2CppObject(self.0), &[$($arg_name),*]);
                result.map_err(|e| anyhow::anyhow!("failed to invoke {}. {:?}", stringify!($func_name), e))
            }
        }
    };
}

#[inline]
pub fn get_cached_class(name: &str) -> Option<Il2CppClass> {
    CLASS_TABLE.get().and_then(|table| table.get(name).copied())
}

#[inline]
pub fn get_native_method(signature: &str) -> Option<Il2CppMethod> {
    if !signature.contains('(') {
        return api_table::resolved_method(signature);
    }
    FUNCTIONS_TABLE
        .get()
        .and_then(|map| map.get(signature).copied())
        .or_else(|| {
            FUNCTIONS_TABLE_REFLECTION
                .get()
                .and_then(|map| map.get(signature).copied())
        })
}
