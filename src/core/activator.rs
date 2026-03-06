use crate::{cs_class, cs_method, il2cpp::native::Il2CppObject};


#[repr(transparent)]
#[derive(Debug, Copy, Clone)]
pub struct Activator(pub usize);

impl Activator {
    cs_class!(&String::from("System.Activator"));
    cs_method!(pub create_instance, "CreateInstance", &["System.Type"], Il2CppObject, (value: usize));
    cs_method!(pub create_instance_with_args, "CreateInstance", &["System.Type", "object[]"], Il2CppObject, (value: usize, args: usize));
    cs_method!(pub create_instance_with_nonpublic, "CreateInstance", &["System.Type", "bool"], Il2CppObject, (value: usize, flag: bool));

    pub fn new(ptr: usize) -> Self {
        Activator(ptr)
    }
}