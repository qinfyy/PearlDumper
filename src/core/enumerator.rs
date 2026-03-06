use crate::{cs_class, cs_method_by_return_type, cs_property};

#[repr(transparent)]
#[derive(Debug, Copy, Clone)]
pub struct FixEnumerator(pub usize);

impl FixEnumerator {
    cs_class!(&String::from("RPG.GameCore.FixEnumerator`2"));

    pub fn new(ptr: usize) -> Self {
        FixEnumerator(ptr)
    }
}