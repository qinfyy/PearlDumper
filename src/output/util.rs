use std::{collections::HashMap, error::Error};

use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};

use crate::{core::{json_serializer::JsonSerializer, json_setting::{JsonSerializerSettings, KeyValuePairConverter, StringEnumConverter}, object::SystemObject, runtime::{MonoField, RuntimeType}}, il2cpp::native::{Il2CppObject, NativeArray}};

fn get_system_type_map() -> HashMap<String, String> {
    let mut m = HashMap::new();
    m.insert("System.Int32".to_string(), "int32".to_string());
    m.insert("System.UInt32".to_string(), "uint32".to_string());
    m.insert("System.Int16".to_string(), "short".to_string());
    m.insert("System.UInt16".to_string(), "ushort".to_string());
    m.insert("System.Int64".to_string(), "int64".to_string());
    m.insert("System.UInt64".to_string(), "uint64".to_string());
    m.insert("System.Byte".to_string(), "byte".to_string());
    m.insert("System.SByte".to_string(), "sbyte".to_string());
    m.insert("System.Boolean".to_string(), "bool".to_string());
    m.insert("System.Single".to_string(), "float".to_string());
    m.insert("System.Double".to_string(), "double".to_string());
    m.insert("System.String".to_string(), "string".to_string());
    m.insert("System.Char".to_string(), "char".to_string());
    m.insert("System.Object".to_string(), "object".to_string());
    m.insert("System.Void".to_string(), "void".to_string());
    m
}
static PRIMITIVE_TYPES: &[&str] = &[
    "Int32",
    "UInt32",
    "Int16",
    "UInt16",
    "Int64",
    "UInt64",
    "Byte",
    "SByte",
    "Boolean",
    "Single",
    "Double",
    "String",
    "Char",
];

pub fn get_field_type_map() -> HashMap<String, String> {
    let mut m = HashMap::new();
    m.insert("MapField<".to_string(), "map<".to_string());
    m.insert("ByteString".to_string(), "bytes".to_string());
    m.insert("RepeatedField<".to_string(), "repeated ".to_string());
    m
}

pub fn contains(s: &str, sub: &str) -> bool {
    s.contains(sub)
}

pub fn count_occurrences(s: &str, sub: &str) -> usize {
    s.matches(sub).count()
}

pub fn get_reflected_type(t: &RuntimeType) -> Result<String, Box<dyn Error>> {
    let s_name = t.get_name()?;
    let name = s_name.as_str().to_string();
    let is_generic = t.get_is_generic_type()?;

    let rt = t.get_reflected_type()?;
    if !is_generic && rt.0 != 0 {
        let parent = get_reflected_type(&rt)?;
        return Ok(format!("{}.{}", parent, name));
    }
    Ok(name)
}

pub fn get_runtime_type_name(t: &RuntimeType, alias: bool) -> Result<String, Box<dyn Error>> {
    if t.0 == 0 {
        return Ok(String::new());
    }
    if t.get_is_array()? {
        let elem = t.get_element_type()?;
        let mut out = get_runtime_type_name(&elem, alias)?;
        let rank = t.get_array_rank()?;
        out.push('[');
        out.push_str(&",".repeat(rank.saturating_sub(1)));
        out.push(']');
        return Ok(out);
    }

    if t.get_is_pointer()? {
        let elem = t.get_element_type()?;
        return Ok(format!("{}*", get_runtime_type_name(&elem, alias)?));
    }

    if t.get_is_by_ref()? {
        let elem = t.get_element_type()?;
        return Ok(format!("{}&", get_runtime_type_name(&elem, alias)?));
    }

    if t.get_is_generic_type()? {
        let s_name = t.get_name()?;
        let mut name = s_name.as_str().to_string();
        if let Some(idx) = name.find('`') {
            name = name[..idx].to_string();
        }

        let args = t.get_generic_arguments()?;
        let mut parts = Vec::new();
        for i in 0..args.len() {
            parts.push(get_runtime_type_name(args.get(i), alias)?);
        }

        return Ok(format!("{}<{}>", name, parts.join(", ")));
    }

    if alias {
        let n_obj = Il2CppObject::new(t.0);
        let s_namespace = n_obj.get_class().namespace();
        if s_namespace.as_str() == "System" {
            let s_full_name = t.get_full_name()?;
            if let Some(mapped) = get_system_type_map().get(&s_full_name.as_str().to_string()) {
                return Ok(mapped.clone());
            }
        }
    }

    get_reflected_type(t)
}

static SKIP_TYPES: &[&str] = &[
    "DynamicFloat",
    "LAHCFFKCOBC",
    "HoyoTagContainer",
];

pub fn search_and_replace_dyn_float(obj: SystemObject) -> Result<(), Box<dyn Error>> {
    if obj.0 == 0 {
        return Ok(());
    }

    let class_type = obj.get_type()?;
    
    let class_name = get_runtime_type_name(&class_type, false)?;
    if PRIMITIVE_TYPES.contains(&class_name.as_str()) {
        return Ok(())
    }

    let fields = class_type.get_fields(20)?;
    for i in 0..fields.len() {
        if fields.get(i) == 0 {
            continue;
        }
        let field = fields.get::<MonoField>(i);

        let value = field.get_value(obj.0)?;
        if value.0 == 0 {
            continue;
        }
  
        let field_type_name = get_runtime_type_name(&field.get_field_type()?, false)?;
        if SKIP_TYPES.contains(&field_type_name.as_str()) {
            let ptr: *const std::ffi::c_void = std::ptr::null();
            field.set_value(obj.0, ptr as usize)?;
        } else if field_type_name.contains("[]") {
            unsafe {
                let arr = &*(value.0 as *const NativeArray<usize>);
                for j in 0..arr.max_length as usize {
                    let item = arr.vector[j];
                    search_and_replace_dyn_float(SystemObject::new(item))?;
                }
            }
        }
        // else if field_type_name.contains("Dictionary") {
        //     let ptr: *const std::ffi::c_void = std::ptr::null();
        //     field.set_value(obj.0, ptr as usize)?;
            // unsafe {
            //     let dict = &*(value.0 as *const NativeDictionary<usize, usize>);
            //     if let Some(entries) = dict.entries.as_ref() {
            //         for i in 0..dict.count as usize {
            //             let entry = entries.vector[i];
            //             let value_dict_ptr = entry.value;
            //             if value_dict_ptr == 0 {
            //                 continue;
            //             }
            //             let _value_dict_type = SystemObject::new(value_dict_ptr).get_type();
            //             let value_dict_type = match _value_dict_type {
            //                 Ok(t) => t,
            //                 Err(_) => {
            //                     continue
            //                 },
            //             };
            //             let value_dict_type_name = get_runtime_type_name(&value_dict_type, false)?;
            //             if PRIMITIVE_TYPES.contains(&value_dict_type_name.as_str()) {
            //                 continue;
            //             }
            //             if value_dict_type_name == "DynamicFloat".to_string() {
            //                 NativeDictionary::<usize, usize>::remove(value.0, entry.key, value_dict_type_name.clone().as_str())?;
            //             }
            //             search_and_replace_dyn_float(SystemObject::new(value_dict_ptr))?;
            //         }
            //     }
            // }
        // }
        else {
            search_and_replace_dyn_float(SystemObject::new(value.0))?;
        }
    }
    Ok(())
}


pub fn setup_json_serializer(formatting: i32, null_handling: i32, reference_loop_handling: i32, default_handling: i32, type_name_handling: i32) -> Result<JsonSerializer, Box<dyn Error>> {
    let json_setting = JsonSerializerSettings::new();
    json_setting.constructor()?;
    let enum_converter = StringEnumConverter::new();
    enum_converter.constructor()?;
    let key_value_converter = KeyValuePairConverter::new();
    key_value_converter.constructor()?;
    let convert_list = json_setting.get_converters()?;
    convert_list.add(
        Il2CppObject::new(enum_converter.0),
        "Newtonsoft.Json.JsonConverter",
    )?;
    convert_list.add(
        Il2CppObject::new(key_value_converter.0),
        "Newtonsoft.Json.JsonConverter",
    )?;
    json_setting.set_converters(convert_list.0)?;
    let json_serializer: JsonSerializer = JsonSerializer::create_default(json_setting)?;
    if formatting != 0 {
        json_serializer.set_formatting(formatting)?;
    }
    if null_handling != 0 {
        json_serializer.set_null_handling(null_handling)?;
    }
    if reference_loop_handling != 0 {
        json_serializer.set_reference_loop_handling(reference_loop_handling)?;
    }
    if default_handling != 0 {
        json_serializer.set_default_handling(default_handling)?;
    }
    if type_name_handling != 0 {
        json_serializer.set_type_name_handling(type_name_handling)?;
    }
    Ok(json_serializer)
}

pub fn create_progress_bar(length: u64, setting: &str) -> ProgressBar {
    let pb = ProgressBar::new(length);
    pb.set_style(
        ProgressStyle::with_template(
            setting,
        )
        .unwrap()
        .progress_chars("█░ "),
    );

    pb
}

pub fn report_errors(error_count: i32, message: &str) {
    if error_count > 0 {
        println!(
            "{} : {}",
            format!("[-] Some errors occurred during {} | Error count: {}", message, error_count).red().bold(),
            error_count
        );
    }
}

pub static YELLOW_PROGRESS_BAR: &str = "[{elapsed_precise}] [{bar:40.yellow/white}] {pos}/{len} {msg}";
pub static GREEN_PROGRESS_BAR: &str = "[{elapsed_precise}] [{bar:40.green/white}] {pos}/{len} {msg}";
pub static BLUE_PROGRESS_BAR: &str = "[{elapsed_precise}] [{bar:40.blue/white}] {pos}/{len} {msg}";
pub static RED_PROGRESS_BAR: &str = "[{elapsed_precise}] [{bar:40.red/white}] {pos}/{len} {msg}";

