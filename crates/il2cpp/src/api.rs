use std::time::{Duration, Instant};

use crate::vm::domain::Il2CppDomain;

pub const REQUIRED_SLOTS: &[usize] = &[
    14, 18, 22, 31, 32, 33, 34, 35, 36, 37, 39, 40, 41, 42, 43, 45, 46, 47, 48, 49, 51, 53, 54, 55,
    56, 57, 63, 64, 65, 73, 75, 76, 77, 116, 117, 122, 123, 124, 125, 128, 129, 130, 131, 132, 140,
    154, 161, 162, 163, 164, 165, 168, 169, 170,
];

pub fn function_address(index: usize) -> anyhow::Result<usize> {
    anyhow::ensure!(index <= 170, "unsupported IL2CPP API slot {index}");
    let slot = (*crate::UP_BASE)
        .checked_add(*crate::API_BASE_PTR)
        .and_then(|base| {
            index
                .checked_mul(8)
                .and_then(|offset| base.checked_add(offset))
        })
        .ok_or_else(|| anyhow::anyhow!("API table address overflow"))?;
    let pointer = unsafe { utils::memory::read::<usize>(slot) }.map_err(|error| {
        anyhow::anyhow!("cannot read API slot {index} at 0x{slot:X}: {error:#}")
    })?;
    anyhow::ensure!(
        utils::memory::is_executable(pointer),
        "API slot {index} is not executable (0x{pointer:X})"
    );
    Ok(pointer)
}

pub fn wait_until_ready(timeout: Duration) -> anyhow::Result<Il2CppDomain> {
    let start = Instant::now();
    let mut last_log = start;
    let mut first_poll = true;
    let mut slots_announced = false;
    log::info!(
        "Waiting for IL2CPP: API table UnityPlayer+0x{:X}, {} required slots, timeout {:.0}s",
        *crate::API_BASE_PTR,
        REQUIRED_SLOTS.len(),
        timeout.as_secs_f64()
    );
    loop {
        let report = first_poll || last_log.elapsed() >= Duration::from_secs(2);
        let readiness = REQUIRED_SLOTS
            .iter()
            .try_for_each(|&slot| function_address(slot).map(|_| ()));
        let mut waiting_on = readiness
            .as_ref()
            .err()
            .map_or_else(String::new, ToString::to_string);
        if readiness.is_ok() {
            if !slots_announced {
                log::info!("IL2CPP API slots ready; querying domain and assemblies");
                slots_announced = true;
            }
            let (domain, assemblies, count) = microseh::try_seh(|| {
                if report {
                    log::info!("Calling il2cpp_domain_get (slot 63)");
                }
                let domain = il2cpp_domain_get();
                if report {
                    log::info!("il2cpp_domain_get returned 0x{:X}", domain.0);
                }
                let mut count = 0;
                let assemblies = if domain.0 == 0 {
                    std::ptr::null_mut()
                } else {
                    if report {
                        log::info!("Calling il2cpp_domain_get_assemblies (slot 65)");
                    }
                    il2cpp_domain_get_assemblies(domain, &mut count)
                };
                if report {
                    log::info!(
                        "Assembly query returned count={count}, array=0x{:X}",
                        assemblies as usize
                    );
                }
                (domain, assemblies, count)
            })
            .map_err(|error| {
                anyhow::anyhow!("native exception while checking IL2CPP readiness: {error:?}")
            })?;
            if domain.0 != 0 && count != 0 {
                anyhow::ensure!(
                    count <= 1024
                        && utils::memory::is_readable(
                            assemblies as usize,
                            count * size_of::<usize>()
                        ),
                    "invalid assembly list during startup (count {count})"
                );
                log::info!(
                    "IL2CPP ready in {:.2}s: domain=0x{:X}, assemblies={count}",
                    start.elapsed().as_secs_f64(),
                    domain.0
                );
                return Ok(domain);
            }
            waiting_on = format!("domain=0x{:X}, assemblies={count}", domain.0);
        }
        if report {
            log::info!(
                "Still waiting for IL2CPP after {:.1}s: {waiting_on}",
                start.elapsed().as_secs_f64()
            );
            last_log = Instant::now();
            first_poll = false;
        }
        if start.elapsed() >= timeout {
            anyhow::bail!("timed out waiting for IL2CPP readiness: {waiting_on}");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

pub use super::vm::{
    array::*, assembly::*, class::*, field::*, image::*, method::*, object::*, thread::*, r#type::*,
};

macro_rules! il2cpp_api {
    ($index:expr, $name:ident($($arg_name:ident: $arg_type:ty),*) -> $ret_type:ty) => {
        #[allow(warnings)]
        #[allow(clippy::not_unsafe_ptr_arg_deref)]
        #[inline(always)]
        pub fn $name($($arg_name: $arg_type,)*) -> $ret_type {
            use std::sync::Once;
            static ONCE: Once = Once::new();

            unsafe {
                let rva = (*super::API_BASE_PTR) + 8 * $index;
                let address = function_address($index).expect("invalid IL2CPP API slot");
                ONCE.call_once(|| {
                    log::debug!(
                        "{}: slot {}, UnityPlayer+0x{rva:X} -> 0x{address:X}",
                        stringify!($name), $index
                    );
                });

                type FuncType = unsafe extern "C" fn($($arg_type,)*) -> $ret_type;
                ::std::mem::transmute::<usize, FuncType>(address)($($arg_name,)*)
            }
        }
    };

    (
        $index:expr,
        $func_name:ident < $($generic:ident : $bound:tt),+ > ($($arg_name:ident: $arg_type:ty),*) -> $ret_type:ty
    ) => {
        #[inline(always)]
        pub fn $func_name < $($generic : $bound),+ > ($($arg_name: $arg_type),*) -> $ret_type {
            use std::sync::Once;
            static ONCE: Once = Once::new();

            unsafe {
                let rva = (*super::API_BASE_PTR) + 8 * $index;
                let address = function_address($index).expect("invalid IL2CPP API slot");
                ONCE.call_once(|| {
                    log::debug!(
                        "{}: slot {}, UnityPlayer+0x{rva:X} -> 0x{address:X}",
                        stringify!($func_name), $index
                    );
                });

                ::std::mem::transmute::<usize, extern "C" fn($($arg_type),*) -> $ret_type>(
                    address
                )($($arg_name,)*)
            }
        }
    };
}

il2cpp_api!(14, il2cpp_array_class_get(element_class: Il2CppClass, rank: u32) -> Il2CppClass);
il2cpp_api!(18, il2cpp_array_new_specific(arrayTypeInfo: Il2CppClass, length: usize) -> Il2CppArray);
il2cpp_api!(22, il2cpp_assembly_get_image(assembly: Il2CppAssembly) -> Il2CppImage);
il2cpp_api!(31, il2cpp_class_get_fields(klass: Il2CppClass, iter: *mut *const usize) -> Il2CppField);
il2cpp_api!(32, il2cpp_class_get_nested_types(klass: Il2CppClass, iter: *const *const usize) -> Il2CppClass);
il2cpp_api!(33, il2cpp_class_get_interfaces(klass: Il2CppClass, iter: *const *const usize) -> Il2CppClass);
il2cpp_api!(34, il2cpp_class_get_field_from_name(klass: Il2CppClass, name: *const i8) -> Il2CppField);
il2cpp_api!(35, il2cpp_class_get_methods(klass: Il2CppClass, iter: *mut *const usize) -> Il2CppMethod);
il2cpp_api!(36, il2cpp_class_get_method_from_name(klass: Il2CppClass, name: *const i8, argsCount: i32) -> Il2CppMethod);
il2cpp_api!(37, il2cpp_class_get_name(klass: Il2CppClass) -> *const i8);
il2cpp_api!(39, il2cpp_class_get_namespace(klass: Il2CppClass) -> *const i8);
il2cpp_api!(40, il2cpp_class_get_parent(klass: Il2CppClass) -> Il2CppClass);
il2cpp_api!(41, il2cpp_class_get_declaring_type(klass: Il2CppClass) -> Il2CppClass);
il2cpp_api!(42, il2cpp_class_instance_size(klass: Il2CppClass) -> i32);
il2cpp_api!(43, il2cpp_class_is_valuetype(klass: Il2CppClass) -> bool);
il2cpp_api!(45, il2cpp_class_get_flags(klass: Il2CppClass) -> i32);
il2cpp_api!(46, il2cpp_class_is_abstract(klass: Il2CppClass) -> bool);
il2cpp_api!(47, il2cpp_class_is_interface(klass: Il2CppClass) -> bool);
il2cpp_api!(48, il2cpp_class_array_element_size(klass: Il2CppClass) -> i32);
il2cpp_api!(49, il2cpp_class_from_type(r#type: Il2CppType) -> Il2CppClass);
il2cpp_api!(51, il2cpp_class_get_type(klass: Il2CppClass) -> Il2CppType);
il2cpp_api!(53, il2cpp_class_is_enum(klass: Il2CppClass) -> bool);
il2cpp_api!(54, il2cpp_class_get_image(klass: Il2CppClass) -> Il2CppImage);
il2cpp_api!(55, il2cpp_class_get_assemblyname(klass: Il2CppClass) -> *const i8);
il2cpp_api!(56, il2cpp_class_get_rank(klass: Il2CppClass) -> i32);
il2cpp_api!(57, il2cpp_class_get_data_size(klass: Il2CppClass) -> u32);
il2cpp_api!(63, il2cpp_domain_get() -> Il2CppDomain);
il2cpp_api!(64, il2cpp_domain_assembly_open(domain: Il2CppDomain, name: *const i8) -> Il2CppAssembly);
il2cpp_api!(65, il2cpp_domain_get_assemblies(domain: Il2CppDomain, size: *mut usize) -> *mut Il2CppAssembly);
il2cpp_api!(73, il2cpp_field_get_name(field: Il2CppField) -> *const i8);
il2cpp_api!(75, il2cpp_field_get_offset(field: Il2CppField) -> usize);
il2cpp_api!(76, il2cpp_field_get_type(field: Il2CppField) -> Il2CppType);
il2cpp_api!(77, il2cpp_field_get_value_object(field: Il2CppField, obj: Il2CppObject) -> Il2CppObject);
il2cpp_api!(116, il2cpp_method_get_return_type(method: Il2CppMethod) -> Il2CppType);
il2cpp_api!(117, il2cpp_method_get_name(method: Il2CppMethod) -> *const i8);
il2cpp_api!(122, il2cpp_method_is_instance(method: Il2CppMethod) -> bool);
il2cpp_api!(123, il2cpp_method_get_param_count(method: Il2CppMethod) -> u32);
il2cpp_api!(124, il2cpp_method_get_param(method: Il2CppMethod, index: u32) -> Il2CppType);
il2cpp_api!(125, il2cpp_method_get_class(method: Il2CppMethod) -> Il2CppClass);
il2cpp_api!(128, il2cpp_object_get_size(obj: Il2CppObject) -> u32);
il2cpp_api!(129, il2cpp_object_get_virtual_method(obj: Il2CppObject, method: Il2CppMethod) -> Il2CppMethod);
il2cpp_api!(130, il2cpp_object_new(klass: Il2CppClass) -> Il2CppObject);
il2cpp_api!(131, il2cpp_object_unbox(obj: Il2CppObject) -> *mut ::std::os::raw::c_void);
il2cpp_api!(132, il2cpp_value_box<T: Copy>(klass: Il2CppClass, data: *const T) -> Il2CppObject);
il2cpp_api!(140, il2cpp_runtime_invoke(
    method: Il2CppMethod,
    obj: Il2CppObject,
    params: *const usize,
    exception: &mut usize
) -> usize);
il2cpp_api!(154, il2cpp_thread_attach(domain: Il2CppDomain) -> Il2CppThread);
il2cpp_api!(161, il2cpp_type_get_name(r#type: Il2CppType) -> *const i8);
il2cpp_api!(162, il2cpp_type_is_byref(r#type: Il2CppType) -> bool);
il2cpp_api!(163, il2cpp_type_get_attrs(r#type: Il2CppType) -> u32);
il2cpp_api!(164, il2cpp_type_equals(r#type: Il2CppType, othertype: Il2CppType) -> bool);
il2cpp_api!(165, il2cpp_type_get_assembly_qualified_name(r#type: Il2CppType) -> *const i8);
il2cpp_api!(168, il2cpp_image_get_name(image: Il2CppImage) -> *const i8);
il2cpp_api!(169, il2cpp_image_get_class_count(image: Il2CppImage) -> usize);
il2cpp_api!(170, il2cpp_image_get_class(image: Il2CppImage, index: usize) -> Il2CppClass);
