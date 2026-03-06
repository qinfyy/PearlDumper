use crate::{core::string::SystemString, cs_class, cs_method};


#[repr(transparent)]
#[derive(Debug, Copy, Clone)]
pub struct MiniJson(pub usize);

impl MiniJson {
    cs_class!(&String::from("MiniJSON_Min.Json"));
    cs_method!(pub serialize, "Serialize", &["object"], SystemString, (value: usize));
    cs_method!(pub deserialize, "Deserialize", &["string"], usize, (value: SystemString));

    pub fn new(ptr: usize) -> Self {
        MiniJson(ptr)
    }
}