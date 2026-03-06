use crate::{core::{string::SystemString}, cs_class, cs_method_custom,  il2cpp::native::Il2CppArray};


#[repr(transparent)]
#[derive(Debug, Copy, Clone)]
pub struct MonoAssembly(pub usize);

impl MonoAssembly {
    cs_class!(&String::from("System.Reflection.Assembly"));
    cs_method_custom!(pub get_full_name, "get_FullName", &[], &String::from("System.Reflection.Assembly"), SystemString, (), self);
    cs_method_custom!(pub get_types, "GetTypes", &["bool"], &String::from("System.Reflection.Assembly"), Il2CppArray, (flags: bool), self);

    pub fn new(ptr: usize) -> Self {
        MonoAssembly(ptr)
    }
}

pub struct Assembly(pub usize);