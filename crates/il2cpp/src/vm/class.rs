use super::{field::Il2CppField, image::Il2CppImage, method::Il2CppMethod, r#type::Il2CppType};
use crate::{api, vm::object::Il2CppObject};
use std::{borrow::Cow, ptr};

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct Il2CppClass(pub usize);

#[allow(unused)]
impl Il2CppClass {
    #[inline]
    pub fn get_name(&self) -> Cow<'static, str> {
        unsafe { utils::cstr_to_str(api::il2cpp_class_get_name(*self)) }
    }

    #[inline]
    pub fn byval_arg(&self) -> Il2CppType {
        api::il2cpp_class_get_type(*self)
    }

    #[inline]
    pub fn get_parent(&self) -> Il2CppClass {
        api::il2cpp_class_get_parent(*self)
    }

    #[inline]
    pub fn get_image(&self) -> Il2CppImage {
        api::il2cpp_class_get_image(*self)
    }

    #[inline]
    pub fn get_namespace(&self) -> Cow<'static, str> {
        unsafe { utils::cstr_to_str(api::il2cpp_class_get_namespace(*self)) }
    }

    #[inline]
    pub fn get_array_class(&self, rank: u32) -> Il2CppClass {
        api::il2cpp_array_class_get(*self, rank)
    }

    pub fn get_fields(&self) -> Vec<Il2CppField> {
        let mut field_iter = ptr::null();
        let mut out: Vec<Il2CppField> = vec![];
        loop {
            unsafe {
                let field = api::il2cpp_class_get_fields(*self, &mut field_iter);

                if field.0 == 0 {
                    break;
                }

                out.push(field);
                assert!(
                    out.len() <= 65536,
                    "IL2CPP field iteration did not terminate"
                );
            }
        }
        out
    }

    pub fn get_methods(&self) -> Vec<Il2CppMethod> {
        let mut iter = ptr::null();
        let mut out: Vec<Il2CppMethod> = vec![];
        loop {
            unsafe {
                let method = api::il2cpp_class_get_methods(*self, &mut iter);

                if method.0 == 0 {
                    break;
                }

                out.push(method);
                assert!(
                    out.len() <= 65536,
                    "IL2CPP method iteration did not terminate"
                );
            }
        }
        out
    }

    pub fn get_interfaces(&self) -> Vec<Il2CppClass> {
        let mut iter = ptr::null();
        let mut out = Vec::new();
        loop {
            let interface = api::il2cpp_class_get_interfaces(*self, &mut iter);
            if interface.0 == 0 {
                break;
            }
            out.push(interface);
            assert!(
                out.len() <= 65536,
                "IL2CPP interface iteration did not terminate"
            );
        }
        out
    }

    #[inline]
    pub fn get_flags(&self) -> i32 {
        api::il2cpp_class_get_flags(*self)
    }

    #[inline]
    pub fn is_interface(&self) -> bool {
        api::il2cpp_class_is_interface(*self)
    }

    #[inline]
    pub fn is_valuetype(&self) -> bool {
        api::il2cpp_class_is_valuetype(*self)
    }

    #[inline]
    pub fn is_enum(&self) -> bool {
        api::il2cpp_class_is_enum(*self)
    }
}

impl Il2CppClass {
    #[inline(always)]
    pub fn create_instance(&self) -> Il2CppObject {
        Il2CppObject::new_object(*self)
    }
}
