
use crate::{ cs_class, cs_method_custom };

#[derive(Debug, Copy, Clone)]
#[repr(transparent)]
pub struct SystemArray(pub usize);

impl SystemArray {
    cs_class!(&String::from("System.Array"));
    cs_method_custom!(pub get_length, "get_Length", &[], &String::from("System.Array"), i32, (), self);
    cs_method_custom!(pub get_value, "GetValue", &["int"], &String::from("System.Array"), usize, (value: i32), self);
}

