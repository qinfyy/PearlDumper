use crate::{cs_class, cs_method};

#[repr(transparent)]
#[derive(Debug, Copy, Clone)]
pub struct SystemType(pub usize);

impl SystemType {
    cs_class!(&String::from("System.Type"));
    cs_method!(pub get_type_from_handle, "GetTypeFromHandle", &["System.RuntimeTypeHandle"], Self, (ty: crate::il2cpp::api::Il2CppType));
}