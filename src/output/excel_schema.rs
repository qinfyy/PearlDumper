use std::{error::Error, fs};

use colored::Colorize;

use crate::{
    core::{
        assembly::MonoAssembly,
        number::{SystemDynamic},
        runtime::{MonoField, RuntimeType},
        string::SystemString,
    },
    il2cpp::native::{Il2CppObject, NativeArray},
    output::util::{create_progress_bar, YELLOW_PROGRESS_BAR},
};

#[derive(Debug, Clone, serde::Serialize)]
pub struct ExcelPath {
    pub paths: Vec<String>,
    pub class_name: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ExcelClassField {
    pub name: String,
    pub class_name: String,
    pub metadata_token: i32,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ExcelClass {
    pub class_name: String,
    pub class_type: String,
    pub parent_class_name: Option<String>,
    pub fields: Vec<ExcelClassField>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Value {
    #[serde(rename = "System.SByte")]
    SByte(i8),
    #[serde(rename = "System.Int16")]
    Int16(i16),
    #[serde(rename = "System.Int32")]
    Int32(i32),
    #[serde(rename = "System.Int64")]
    Int64(i64),
    #[serde(rename = "System.Byte")]
    Byte(u8),
    #[serde(rename = "System.UInt16")]
    UInt16(u16),
    #[serde(rename = "System.UInt32")]
    UInt32(u32),
    #[serde(rename = "System.UInt64")]
    UInt64(u64),
}


#[derive(Debug, Clone, serde::Serialize)]
pub struct ExcelEnumField {
    pub name: String,
    pub value: Value,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ExcelEnum {
    pub class_name: String,
    pub value_type: String,
    pub fields: Vec<ExcelEnumField>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ExcelSchema {
    pub list_path: Vec<ExcelPath>,
    pub list_type: Vec<ExcelClass>,
    pub list_enum: Vec<ExcelEnum>,
}

fn handle_path(runtime_type: RuntimeType) -> Result<ExcelPath, Box<dyn Error>> {
    let mut paths: Vec<String> = vec![];
    let mut class_name = String::new();
    let fields = runtime_type.get_fields(60)?;
    for i in 0..fields.len() {
        let field= fields.get::<MonoField>(i);
        let field_name = field.get_name()?.as_str();
        if field_name == "s_PathList" || field_name == "PIPMCNKGEKM" {
            unsafe {
                let ptr: *const std::ffi::c_void = std::ptr::null();
                let _spath = field.get_value(ptr as usize);
                let spath = match _spath {
                    Ok(path) => path,
                    Err(e) => {
                        println!("Error: {}", e);
                        continue;
                    }
                };

                let arr_native = &*(spath.0 as *const NativeArray<SystemString>);

                for j in 0..arr_native.max_length as usize {
                    let item = arr_native.vector[j];
                    paths.push(item.as_str().to_string());
                }
            }
        }

        if field_name == "dataDict_" || field_name == "FHBDBNIPGAO" {
            class_name = field
                .get_field_type()?
                .get_il2cpp_type()
                .name()
                .split('<')
                .nth(1)
                .unwrap()
                .trim_end_matches('>')
                .split(',')
                .nth(1)
                .unwrap()
                .trim()
                .to_string();
        }
    }

    if !paths.is_empty() && !class_name.is_empty() {
        Ok(ExcelPath { paths, class_name })
    } else {
        Err("No paths or class name found".into())
    }
}

fn handle_class(runtime_type: RuntimeType) -> Result<ExcelClass, Box<dyn Error>> {
    if runtime_type.get_is_interface()? {
        return Err("Interface not supported".into());
    }
    let fields = runtime_type.get_field_special()?;
    let mut class_fields: Vec<ExcelClassField> = vec![];
    for field in fields {
        class_fields.push(ExcelClassField {
            name: field.get_name()?.as_str().to_string(),
            class_name: field.get_field_type()?.get_il2cpp_type().name(),
            metadata_token: field.get_metadata_token()?,
        });
    }
    let mut parent_class_name = None;
    let parent_class = runtime_type.get_il2cpp_type().get_class().get_parent().byval_arg();
    if parent_class.0 != 0 {
        parent_class_name = Some(parent_class.name());
    }
    let class_type = if runtime_type.get_is_value_type()? {
        "struct"
    } else {
        "class"
    };

    Ok(ExcelClass {
        class_name: runtime_type.get_il2cpp_type().name(),
        parent_class_name,
        class_type: class_type.to_string(),
        fields: class_fields
    })
}

fn parse_value(object_type: &str, val_str: &str) -> Result<Value, Box<dyn std::error::Error>> {
    match object_type {
        "Byte"   => Ok(Value::Byte(val_str.parse::<u8>()?)),
        "UInt16" => Ok(Value::UInt16(val_str.parse::<u16>()?)),
        "UInt32" => Ok(Value::UInt32(val_str.parse::<u32>()?)),
        "UInt64" => Ok(Value::UInt64(val_str.parse::<u64>()?)),
        "SByte"  => Ok(Value::SByte(val_str.parse::<i8>()?)),
        "Int16"  => Ok(Value::Int16(val_str.parse::<i16>()?)),
        "Int32"  => Ok(Value::Int32(val_str.parse::<i32>()?)),
        "Int64"  => Ok(Value::Int64(val_str.parse::<i64>()?)),
        _        => Err(format!("Unsupported objectType: {}", object_type).into()),
    }
}

fn handle_enum(runtime_type: RuntimeType) -> Result<ExcelEnum, Box<dyn Error>> {
    let fields = runtime_type.get_fields(60)?;
    let mut enum_fields: Vec<ExcelEnumField> = vec![];
    let mut value_type = String::new();
    for i in 0..fields.len() {
        let field = fields.get::<MonoField>(i);

        if field.get_name()?.as_str() != "value__" {
            let int_ptr = field.get_raw_constant_value()?;
            let val_str = SystemDynamic::new(int_ptr).get_string()?.as_str().to_string();
            let field_value_type = Il2CppObject::new(int_ptr).get_class().name();
            let value = parse_value(field_value_type.as_str(), &val_str)?;
            value_type = format!("System.{}", field_value_type);
            enum_fields.push(ExcelEnumField {
                name: field.get_name()?.as_str().to_string(),
                value,
            });
        }
    }

    Ok(ExcelEnum {
        class_name: runtime_type.get_il2cpp_type().name(),
        value_type,
        fields: enum_fields,
    })
}

pub fn excel_schema_dump(mono_assembly: &MonoAssembly) -> Result<(), Box<dyn Error>> {
    println!("{}", "📦 Scanning and filtering classes...".green().bold());
    let types = mono_assembly.get_types(60 != 0)?;
    let mut data_json: ExcelSchema = ExcelSchema {
        list_path: vec![],
        list_type: vec![],
        list_enum: vec![],
    };

    let pb_filter = create_progress_bar(types.len() as u64, YELLOW_PROGRESS_BAR);
    for j in 0..types.len() {
        let runtime_ptr: &usize = types.get(j);
        let runtime_type = RuntimeType::new(*runtime_ptr);
        pb_filter.set_message(format!(
            "Scanning type {}",
            runtime_type.get_full_name()?.as_str()
        ));
        if runtime_type.get_il2cpp_type().name().contains("<") {
            continue;
        }
        if runtime_type.get_is_enum()? {
            let result = handle_enum(runtime_type);
            match result {
                Ok(excel_enum) => {
                    data_json.list_enum.push(excel_enum);
                }
                Err(_) => {
                    // println!("{}", e);
                }
            }
        } else {
            let result_path = handle_path(runtime_type);
            match result_path {
                Ok(excel_path) => {
                    data_json.list_path.push(excel_path);
                }
                Err(_) => {
                    // println!("Error: {}", e);
                }
            }

            let result_class = handle_class(runtime_type);
            match result_class {
                Ok(excel_class) => {
                    data_json.list_type.push(excel_class);
                }
                Err(_) => {
                    // println!("{}", e);
                }
            }
        }

        pb_filter.inc(1);
    }
    pb_filter.finish_with_message("✅ Finished scanning and filtering classes.");
    fs::write(
        "ExcelSchema.json",
        serde_json::to_string_pretty(&data_json).unwrap(),
    )
    .unwrap();
    println!(
        "{}",
        "✅ Finished scanning and filtering classes..."
            .green()
            .bold()
    );

    Ok(())
}
