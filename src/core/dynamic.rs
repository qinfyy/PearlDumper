use crate::{cs_class, cs_method_by_return_type, cs_property};

#[repr(transparent)]
#[derive(Debug, Copy, Clone)]
pub struct DynamicFloat(pub usize);

impl DynamicFloat {
    cs_class!(&String::from("RPG.GameCore.DynamicFloat"));
    cs_property!(pub is_dynamic, "get_IsDynamic", bool, self);
    cs_method_by_return_type!(pub create_by_fix_point, "RPG.GameCore.DynamicFloat", &["RPG.GameCore.FixPoint"], DynamicFloat, (value: usize));

    pub fn new(ptr: usize) -> Self {
        DynamicFloat(ptr)
    }
}