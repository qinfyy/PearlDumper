use crate::{core::string::SystemString, cs_class, cs_method, il2cpp::{api::il2cpp_object_new, get_cached_class}};


#[repr(transparent)]
#[derive(Debug, Copy, Clone)]
pub struct JsonConvert(pub usize);

impl JsonConvert {
    cs_class!(&String::from("Newtonsoft.Json.JsonConvert"));
    cs_method!(pub serialize_object, "SerializeObject", &["object"], SystemString, (value: usize));
    cs_method!(pub serialize_object_with_formatting, "SerializeObject", &["object", "Newtonsoft.Json.Formatting"], SystemString, (value: usize, formatting: i32));
    cs_method!(pub serialize_object_with_formatting_and_settings, "SerializeObject", &["object", "Newtonsoft.Json.Formatting", "Newtonsoft.Json.JsonSerializerSettings"], SystemString, (value: usize, formatting: i32, settings: usize));

    pub fn new() -> Self {
        let class = get_cached_class(&String::from("Newtonsoft.Json.JsonConvert")).unwrap();

        let instance = il2cpp_object_new(class);
        JsonConvert(instance.0)
    }
}