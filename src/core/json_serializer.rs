use crate::{core::{json_setting::JsonSerializerSettings, writer::JsonTextWriter}, cs_class, cs_method, cs_property, il2cpp::{api::il2cpp_object_new, get_cached_class}};


#[repr(transparent)]
#[derive(Debug, Copy, Clone)]
pub struct JsonSerializer(pub usize);

impl JsonSerializer {
    cs_class!(&String::from("Newtonsoft.Json.JsonSerializer"));
    cs_property!(pub formatting, "get_Formatting", i32, self);
    cs_property!(pub null_handling, "get_NullValueHandling", i32, self);
    cs_property!(pub default_handling, "get_DefaultValueHandling", i32, self);
    cs_property!(pub reference_loop_handling, "get_ReferenceLoopHandling", i32, self);
    cs_method!(pub create_default, "CreateDefault", &["Newtonsoft.Json.JsonSerializerSettings"], JsonSerializer, (settings: JsonSerializerSettings));
    cs_method!(pub set_formatting, "set_Formatting", &["Newtonsoft.Json.Formatting"], (), (formatting: i32), self);
    cs_method!(pub set_null_handling, "set_NullValueHandling", &["Newtonsoft.Json.NullValueHandling"], (), (null_handling: i32), self);
    cs_method!(pub set_default_handling, "set_DefaultValueHandling", &["Newtonsoft.Json.DefaultValueHandling"], (), (default_handling: i32), self);
    cs_method!(pub set_reference_loop_handling, "set_ReferenceLoopHandling", &["Newtonsoft.Json.ReferenceLoopHandling"], (), (reference_loop_handling: i32), self);
    cs_method!(pub set_type_name_handling, "set_TypeNameHandling", &["Newtonsoft.Json.TypeNameHandling"], (), (type_name_handling: i32), self);
    cs_method!(pub serialize, "Serialize", &["Newtonsoft.Json.JsonWriter", "object"], (), (writer: JsonTextWriter, value: usize), self);

    pub fn new() -> Self {
        let class = get_cached_class(&String::from("Newtonsoft.Json.JsonSerializer")).unwrap();
        let instance = il2cpp_object_new(class);
        JsonSerializer(instance.0)
    }
}