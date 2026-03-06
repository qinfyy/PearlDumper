use crate::{core::string::SystemString, cs_class, cs_property};


#[repr(transparent)]
#[derive(Debug, Copy, Clone)]
pub struct OriginalNameAttribute(pub usize);

impl OriginalNameAttribute {
    cs_class!(&String::from("Google.Protobuf.Reflection.OriginalNameAttribute"));
    cs_property!(pub name, "get_Name", SystemString, self);

    pub fn new(ptr: usize) -> Self {
        OriginalNameAttribute(ptr)
    }
}