
use crate::{core::string::SystemString, cs_class, cs_method, il2cpp::{api::il2cpp_object_new, get_cached_class}};


#[repr(transparent)]
#[derive(Debug, Copy, Clone)]
pub struct StreamWriter(pub usize);

impl StreamWriter {
    cs_class!(&String::from("System.IO.StreamWriter"));
    cs_method!(pub constructor, ".ctor", &["string"], (), (path: SystemString), self);
    cs_method!(pub close, "Close", &[], (), (),self);
    
    pub fn new() -> Self {
        let class = get_cached_class(&String::from("System.IO.StreamWriter")).unwrap();
        let instance = il2cpp_object_new(class);
        StreamWriter(instance.0)
    }
}

#[repr(transparent)]
#[derive(Debug, Copy, Clone)]
pub struct SystemDirectory(pub usize);

impl SystemDirectory {
    cs_class!(&String::from("System.IO.Directory"));
    cs_method!(pub create_directory, "CreateDirectory", &["string"], usize, (path: SystemString));
    cs_method!(pub exists, "Exists", &["string"], bool, (path: SystemString));
}

#[repr(transparent)]
#[derive(Debug, Copy, Clone)]
pub struct JsonTextWriter(pub usize);

impl JsonTextWriter {
    cs_class!(&String::from("Newtonsoft.Json.JsonTextWriter"));
    cs_method!(pub constructor, ".ctor", &["System.IO.TextWriter"], (), (writer: StreamWriter), self);
    cs_method!(pub close, "Close", &[], (), (),self);
    pub fn new() -> Self {
        let class = get_cached_class(&String::from("Newtonsoft.Json.JsonTextWriter")).unwrap();
        let instance = il2cpp_object_new(class);
        JsonTextWriter(instance.0)
    }
}

