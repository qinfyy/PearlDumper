use crate::{core::runtime::RuntimeType, cs_class, cs_method_custom};

#[repr(transparent)]
#[derive(Debug, Copy, Clone)]
pub struct SystemObject(pub usize);

impl SystemObject {
    cs_class!(&String::from("System.Object"));

    cs_method_custom!(pub get_type, "GetType", &[], &String::from("System.Object"), RuntimeType, (), self);

    pub fn new(ptr: usize) -> Self {
        SystemObject(ptr)
    }
}