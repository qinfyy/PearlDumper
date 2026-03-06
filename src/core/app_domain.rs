use crate::{cs_class, cs_method, cs_property, il2cpp::native::Il2CppArray};

#[repr(transparent)]
#[derive(Debug, Copy, Clone)]
pub struct AppDomain(pub usize);

impl AppDomain {
    cs_class!(&String::from("System.AppDomain"));
    cs_property!(pub assemblies, "GetAssemblies", Il2CppArray, self);
    cs_method!(pub get_current_domain, "get_CurrentDomain", &[], AppDomain, ());

    pub fn new(ptr: usize) -> Self {
        AppDomain(ptr)
    }
}

pub struct Assembly(pub usize);