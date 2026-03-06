use crate::{core::string::SystemString, cs_class, cs_property};

#[repr(transparent)]
#[derive(Debug, Copy, Clone)]
pub struct SystemInt32(pub usize);

impl SystemInt32 {
    cs_class!(&String::from("System.Int32"));
    cs_property!(pub string, "ToString", SystemString, self);

    pub fn new(ptr: usize) -> Self {
        SystemInt32(ptr)
    }
}
