use crate::{cs_class, cs_method, cs_property, il2cpp::{api::il2cpp_object_new, get_cached_class, native::NativeList}};


#[repr(transparent)]
#[derive(Debug, Copy, Clone)]
pub struct JsonSerializerSettings(pub usize);

impl JsonSerializerSettings {
    cs_class!(&String::from("Newtonsoft.Json.JsonSerializerSettings"));
    cs_property!(pub converters, "get_Converters", NativeList, self);
    cs_method!(pub set_converters, "set_Converters", &["System.Collections.Generic.IList<Newtonsoft.Json.JsonConverter>"], usize, (converters: usize), self);
    cs_method!(pub constructor, ".ctor", &[], (), (), self);
    
    pub fn new() -> Self {
        let class = get_cached_class(&String::from("Newtonsoft.Json.JsonSerializerSettings")).unwrap();
        let instance = il2cpp_object_new(class);
        JsonSerializerSettings(instance.0)
    }
}


#[repr(transparent)]
#[derive(Debug, Copy, Clone)]
pub struct KeyValuePairConverter(pub usize);

impl KeyValuePairConverter {
    cs_class!(&String::from("Newtonsoft.Json.Converters.KeyValuePairConverter"));
    cs_method!(pub constructor, ".ctor", &[], (), (), self);
    
    pub fn new() -> Self {
        let class = get_cached_class(&String::from("Newtonsoft.Json.Converters.KeyValuePairConverter")).unwrap();
        let instance = il2cpp_object_new(class);
        KeyValuePairConverter(instance.0)
    }
}

#[repr(transparent)]
#[derive(Debug, Copy, Clone)]
pub struct StringEnumConverter(pub usize);

impl StringEnumConverter {
    cs_class!(&String::from("Newtonsoft.Json.Converters.StringEnumConverter"));
    cs_method!(pub constructor, ".ctor", &[], (), (), self);
    
    pub fn new() -> Self {
        let class = get_cached_class(&String::from("Newtonsoft.Json.Converters.StringEnumConverter")).unwrap();
        let instance = il2cpp_object_new(class);
        StringEnumConverter(instance.0)
    }
}

#[repr(transparent)]
#[derive(Debug, Copy, Clone)]
pub struct HashSetConverter(pub usize);

impl HashSetConverter {
    cs_class!(&String::from("Newtonsoft.Json.Converters.HashSetConverter"));
    cs_method!(pub constructor, ".ctor", &[], (), (), self);
    
    pub fn new() -> Self {
        let class = get_cached_class(&String::from("Newtonsoft.Json.Converters.HashSetConverter")).unwrap();
        let instance = il2cpp_object_new(class);
        HashSetConverter(instance.0)
    }
}

#[repr(transparent)]
#[derive(Debug, Copy, Clone)]
pub struct BsonObjectIdConverter(pub usize);

impl BsonObjectIdConverter {
    cs_class!(&String::from("Newtonsoft.Json.Converters.BsonObjectIdConverter"));
    cs_method!(pub constructor, ".ctor", &[], (), (), self);
    
    pub fn new() -> Self {
        let class = get_cached_class(&String::from("Newtonsoft.Json.Converters.BsonObjectIdConverter")).unwrap();
        let instance = il2cpp_object_new(class);
        BsonObjectIdConverter(instance.0)
    }
}

#[repr(transparent)]
#[derive(Debug, Copy, Clone)]
pub struct RegexConverter(pub usize);

impl RegexConverter {
    cs_class!(&String::from("Newtonsoft.Json.Converters.RegexConverter"));
    cs_method!(pub constructor, ".ctor", &[], (), (), self);
    
    pub fn new() -> Self {
        let class = get_cached_class(&String::from("Newtonsoft.Json.Converters.RegexConverter")).unwrap();
        let instance = il2cpp_object_new(class);
        RegexConverter(instance.0)
    }
}

#[repr(transparent)]
#[derive(Debug, Copy, Clone)]
pub struct VersionConverter(pub usize);

impl VersionConverter {
    cs_class!(&String::from("Newtonsoft.Json.Converters.VersionConverter"));
    cs_method!(pub constructor, ".ctor", &[], (), (), self);
    
    pub fn new() -> Self {
        let class = get_cached_class(&String::from("Newtonsoft.Json.Converters.VersionConverter")).unwrap();
        let instance = il2cpp_object_new(class);
        VersionConverter(instance.0)
    }
}

#[repr(transparent)]
#[derive(Debug, Copy, Clone)]
pub struct IsoDateTimeConverter(pub usize);

impl IsoDateTimeConverter {
    cs_class!(&String::from("Newtonsoft.Json.Converters.IsoDateTimeConverter"));
    cs_method!(pub constructor, ".ctor", &[], (), (), self);
    
    pub fn new() -> Self {
        let class = get_cached_class(&String::from("Newtonsoft.Json.Converters.IsoDateTimeConverter")).unwrap();
        let instance = il2cpp_object_new(class);
        IsoDateTimeConverter(instance.0)
    }
}

#[repr(transparent)]
#[derive(Debug, Copy, Clone)]
pub struct JavaScriptDateTimeConverter(pub usize);

impl JavaScriptDateTimeConverter {
    cs_class!(&String::from("Newtonsoft.Json.Converters.JavaScriptDateTimeConverter"));
    cs_method!(pub constructor, ".ctor", &[], (), (), self);
    
    pub fn new() -> Self {
        let class = get_cached_class(&String::from("Newtonsoft.Json.Converters.JavaScriptDateTimeConverter")).unwrap();
        let instance = il2cpp_object_new(class);
        JavaScriptDateTimeConverter(instance.0)
    }
}