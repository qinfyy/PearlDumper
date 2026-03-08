use crate::{ cs_class, cs_field, il2cpp::native::NativeList };

#[repr(transparent)]
#[derive(Debug, Copy, Clone)]
pub struct ModuleManager(pub usize);

impl ModuleManager {
    cs_class!(&String::from("RPG.Client.ModuleManager"));

    cs_field!(modules, "modules", self, |v| -> NativeList { NativeList(v.0) });
    pub fn new(ptr: usize) -> Self {
        ModuleManager(ptr)
    }
}