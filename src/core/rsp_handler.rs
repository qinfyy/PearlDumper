use crate::{ cs_class };

#[repr(transparent)]
#[derive(Debug, Copy, Clone)]
pub struct RspHandler(pub usize);

impl RspHandler {
    cs_class!(&String::from("RPG.Client.RspHandler"));

    pub fn new(ptr: usize) -> Self {
        RspHandler(ptr)
    }
}