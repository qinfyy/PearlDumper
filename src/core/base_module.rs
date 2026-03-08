use crate::{ cs_class, cs_method, cs_method_custom};

#[repr(transparent)]
#[derive(Debug, Copy, Clone)]
pub struct BaseModule(pub usize);

impl BaseModule {
    cs_class!(&String::from("RPG.Client.BaseModule"));
    cs_method_custom!(pub add_packet_handlers_parent, "_AddPacketHandlers", &[], &String::from("RPG.Client.BaseModule"), (), (), self);
    cs_method!(pub add_packet_handlers_child, "_AddPacketHandlers", &[], (), (), self);

    pub fn new(ptr: usize) -> Self {
        BaseModule(ptr)
    }
}