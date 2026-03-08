use crate::{
    core::{
        assembly::MonoAssembly,
        attribute::OriginalNameAttribute,
        number::SystemDynamic,
        runtime::{MonoField, MonoProperty, RuntimeType},
    },
    il2cpp::{api::Il2CppType, get_cached_class, native::Il2CppObject},
    output::{cmdid::get_cmdid, util::{
        BLUE_PROGRESS_BAR, YELLOW_PROGRESS_BAR, contains, count_occurrences, create_progress_bar,
        get_field_type_map, get_runtime_type_name,
    }},
};
use colored::Colorize;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    error::Error,
    fs::File,
    io::{BufWriter, Write},
    thread, time,
};

fn dump_csharp_type(t: &RuntimeType) -> Result<String, Box<dyn std::error::Error>> {
    let mut out = String::new();
    let runtime_type_name = get_runtime_type_name(t, false)?;
    if runtime_type_name.ends_with("Attribute") {
        return Ok(String::new());
    }
    if runtime_type_name.contains('.') {
        return Ok(String::new());
    }
    let mut cmd_name: Option<String> = None;
    let mut skip_types: HashMap<String, bool> = HashMap::new();

    if !runtime_type_name.contains('<') {
        let fields = t.get_fields(60)?;

        let mut inner = String::new();
        let mut is_enum_type = t.get_is_enum()?;
        let mut field_ids: HashMap<i32, String> = HashMap::new();
        let mut idx = 0;
        if fields.len() > 0 {
            for i in 0..fields.len() {
                let field = fields.get::<MonoField>(i);
                let full_name_field = field.get_field_type()?.get_full_name()?.as_str();
                let d_field_name = field.get_declaring_type()?.get_name()?.as_str();
                let mut enum_key: Option<String> = None;
                if d_field_name == t.get_name()?.as_str() {
                    let attrs = field.get_custom_attribute(true)?;
                    if attrs.len() > 0 {
                        for j in 0..attrs.len() {
                            let n_rt= attrs.get::<Il2CppObject>(j);
                            let name_rt = n_rt.get_class().name();
                            if name_rt == "OriginalNameAttribute".to_string() {
                                is_enum_type = true;

                                enum_key = Some(
                                    OriginalNameAttribute::new(n_rt.0)
                                        .get_name()?
                                        .as_str()
                                        .to_string(),
                                );

                                if cmd_name.is_none() {
                                    if let Some(key) = &enum_key {
                                        if key.starts_with("CMD") {
                                            let re = Regex::new(r"^CMD(.*?)_NONE$").unwrap();
                                            if let Some(caps) = re.captures(&key) {
                                                if let Some(raw) = caps.get(1) {
                                                    let lower = raw.as_str().to_lowercase();
                                                    let capitalized =
                                                        Regex::new(r"_(\w)").unwrap().replace_all(
                                                            &lower,
                                                            |caps: &regex::Captures| {
                                                                caps[1].to_uppercase()
                                                            },
                                                        );
                                                    cmd_name = Some(format!("Cmd{}", capitalized));
                                                }
                                            }
                                        }
                                        if let Some(key) = &enum_key {
                                            if cmd_name.is_none() && key.starts_with("Cmd") {
                                                let re = Regex::new(r"^Cmd(.*?)None$").unwrap();
                                                if let Some(caps) = re.captures(&key) {
                                                    if let Some(raw) = caps.get(1) {
                                                        cmd_name =
                                                            Some(format!("Cmd{}", raw.as_str()));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if attrs.len() == 0 && t.get_is_enum()? && field.get_is_literal()? {
                        is_enum_type = true;
                        enum_key = Some(
                            (t.get_name()?.as_str() + "_" + field.get_name()?.as_str()).to_string(),
                        );
                    }
                }

                if field.get_is_literal()? {
                    let int32_ptr = field.get_raw_constant_value()?;
                    let n_int32 = SystemDynamic::new(int32_ptr);
                    let value = n_int32.get_string()?.as_str().to_string();
                    field_ids.insert(idx, value.clone());
                    idx += 1;
                    if let Some(enum_key) = &enum_key {
                        inner.push_str(&format!("\t{} = {};\n", enum_key, value.clone()));
                    }
                } else if contains(&full_name_field, &d_field_name) {
                    let inner_fields = field.get_field_type()?.get_fields(60)?;
                    let count = count_occurrences(&full_name_field, "+");

                    if count == 1 {
                        inner.push_str(&format!("\toneof {} {{\n", field.get_name()?.as_str()));
                        for j in 0..inner_fields.len() {
                            let one_of_field = inner_fields.get::<MonoField>(j);
                            if one_of_field.get_is_literal()? {
                                let int32_ptr = one_of_field.get_raw_constant_value()?;
                                let value = SystemDynamic::new(int32_ptr)
                                    .get_string()?
                                    .as_str()
                                    .to_string();
                                if value != "" && value != "0" {
                                    inner.push_str("\t\tint32 ");
                                    inner.push_str(&one_of_field.get_name()?.as_str());
                                    inner.push_str(&format!(" = {};", value));
                                    inner.push('\n');
                                }
                                skip_types
                                    .insert(one_of_field.get_name()?.as_str().to_string(), true);
                            }
                        }
                        inner.push_str("\t}\n");
                    } else if count == 2 {
                        inner.push_str(&format!(
                            "\tenum {} {{\n",
                            field.get_field_type()?.get_name()?.as_str()
                        ));

                        for j in 0..inner_fields.len() {
                            let nested_field = inner_fields.get::<MonoField>(j);
                            let attrs = nested_field.get_custom_attribute(true)?;
                            if attrs.len() > 0 {
                                for k in 0..attrs.len() {
                                    let n_rt= attrs.get::<Il2CppObject>(k);
                                    if n_rt.get_class().name()
                                        == "OriginalNameAttribute".to_string()
                                    {
                                        let n_enum_key = Some(
                                            OriginalNameAttribute::new(n_rt.0)
                                                .get_name()?
                                                .as_str()
                                                .to_string(),
                                        );
                                        if nested_field.get_is_literal()? {
                                            let int32_ptr =
                                                nested_field.get_raw_constant_value()?;
                                            let value = SystemDynamic::new(int32_ptr)
                                                .get_string()?
                                                .as_str()
                                                .to_string();
                                            if value != "" {
                                                if let Some(key) = n_enum_key {
                                                    inner.push_str("\t\t");
                                                    inner.push_str(&key);
                                                    inner.push_str(&format!(" = {};", value));
                                                    inner.push('\n');
                                                    skip_types.insert(key, true);
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            if attrs.len() == 0 && nested_field.get_is_literal()? {
                                let n_enum_key = Some(
                                    (field.get_field_type()?.get_name()?.as_str()
                                        + "_"
                                        + nested_field.get_name()?.as_str())
                                    .to_string(),
                                );
                                let int32_ptr = nested_field.get_raw_constant_value()?;
                                let value = SystemDynamic::new(int32_ptr)
                                    .get_string()?
                                    .as_str()
                                    .to_string();
                                if value != "" {
                                    if let Some(key) = n_enum_key {
                                        inner.push_str("\t\t");
                                        inner.push_str(&key);
                                        inner.push_str(&format!(" = {};", value));
                                        inner.push('\n');
                                        skip_types.insert(key, true);
                                    }
                                }
                            }
                        }
                        inner.push_str("\t}\n");
                    }
                }
            }
        }

        out.push_str("\n");

        if let Some(cmd_name) = &cmd_name {
            out.push_str(&format!("// {}\n", runtime_type_name));
            out.push_str(&format!("enum {}", cmd_name));
        } else if is_enum_type {
            out.push_str(&format!("enum {}", runtime_type_name));
        } else {
            out.push_str(&format!("message {}", runtime_type_name));
        }

        out.push_str(" {\n");
        out.push_str(&inner);

        let properties = t.get_properties(28)?;
        if properties.len() > 0 {
            for j in 0..properties.len() {
                let property_ptr: &usize = properties.get(j);
                let property = MonoProperty::new(*property_ptr);
                let d_property_name = property.get_declaring_type()?.get_name()?.as_str();
                if d_property_name == t.get_name()?.as_str() {
                    if let Some(field_id) = field_ids.get(&((j) as i32)) {
                        let mut typename =
                            get_runtime_type_name(&property.get_property_type()?, true)?;
                        if typename.contains('.') {
                            if let Some(capture) = typename.rsplit('.').next() {
                                typename = capture.to_string();
                            }
                        }
                        for (pattern, replacement) in &get_field_type_map() {
                            typename = typename.replace(pattern, replacement);
                        }

                        if typename.contains("repeated") {
                            typename = typename.replace(">", "");
                        }

                        if skip_types.contains_key(&property.get_name()?.as_str().to_string()) {
                            let pattern = format!("int32 {}", property.get_name()?.as_str());
                            let replacement =
                                format!("{} {}", typename, property.get_name()?.as_str());
                            out = out.replace(&pattern, &replacement);
                        } else if field_id != "0" {
                            out += &format!(
                                "\t{} {} = {};\n",
                                typename,
                                property.get_name()?.as_str(),
                                field_id
                            );
                        }
                    }
                }
            }
        }

        out.push_str("}\n");
    }

    Ok(out)
}

const PROTO_FILE: &str = "StarRail_PreTranslate.proto";

#[derive(Serialize, Deserialize)]
struct CmdCache(HashMap<String, u16>);


pub fn proto_dump(mono_assembly: &MonoAssembly) -> Result<(), Box<dyn Error>> {
    println!("{}", "📦 Dumping raw proto....".green().bold());
    let file = File::create("StarRail_Raw.proto")?;
    let mut writer = BufWriter::new(file);

    let types = mono_assembly.get_types(60 != 0)?;
    let mut final_str = String::new();
    final_str.push_str("syntax = \"proto3\";\n");
    final_str.push_str("\npackage starrail;\n");
    final_str.push_str("\n// Castorice Dumper.\n");
    // Progress bar setup
    let pb = create_progress_bar(types.len() as u64, YELLOW_PROGRESS_BAR);

    for j in 0..types.len() {
        let runtime_ptr: &usize = types.get(j);
        let runtime_type = RuntimeType::new(*runtime_ptr);
        pb.set_message(format!(
            "Processing {}",
            runtime_type.get_full_name()?.as_str()
        ));
        if !runtime_type.get_is_generic_type()? {
            let output: String = dump_csharp_type(&runtime_type)?;
            final_str.push_str(&output.clone());
        }

        pb.inc(1);
    }

    pb.finish_with_message("✅ Finished dumping raw proto.");
    writer.write_all(final_str.as_bytes())?;
    writer.flush()?;

    println!("{}", "✅ Finished dumping raw proto.".green().bold());

    println!(
        "{}",
        "📦 Translating names of some proto classes..."
            .green()
            .bold()
    );
    let mut map = get_cmdid();
    let mut error_map: HashMap<String, u16> = HashMap::new();

    let class_cf =
        get_cached_class(&String::from("EIFPLDNKAIB")).ok_or(anyhow::anyhow!("no such class"))?; //4.0.5x

    let pb2 = create_progress_bar(map.len() as u64, BLUE_PROGRESS_BAR);

    while !map.is_empty() {
        let current_map = map.clone();

        for (key, value) in current_map {
            let mut success = false;

            for _attempt in 0..3 {
                unsafe {
                    pb2.set_message(format!("Processing {}", key.clone()));

                    let method = class_cf
                        .find_method_by_return_type("System.RuntimeTypeHandle", &["ushort"])
                        .ok_or(anyhow::anyhow!("no such method"))?;

                    let func_gd: extern "fastcall" fn(u16) -> Il2CppType =
                        std::mem::transmute(method.va());

                    let result = microseh::try_seh(|| func_gd(value.try_into().unwrap()));

                    match result {
                        Ok(rt_class) if rt_class.0 != 0 => {
                            let original = format!("message {} {{", rt_class.name());
                            let replaced = format!(
                                "// Obf: {}\n// CmdID: {}\nmessage {} {{",
                                rt_class.name(),
                                value,
                                key.replace("Cmd", "")
                            );

                            final_str = final_str.replace(&original, &replaced);
                            map.remove(&key);
                            pb2.inc(1);
                            success = true;
                            break;
                        }
                        _ => {
                            thread::sleep(time::Duration::from_millis(200));
                        }
                    }
                }
            }

            if !success {
                map.remove(&key);
                error_map.insert(key, value.try_into().unwrap());
            }
        }
    }
    pb2.finish_with_message("✅ Finished translating names of some proto classes.");

    if !error_map.is_empty() {
        let json = serde_json::to_string_pretty(&CmdCache(error_map.clone()))?;
        std::fs::write("error_cmd.json", json)?;
        println!(
            "{} | Error count: {}",
            "❌ Some proto classes could not be translated."
                .red()
                .bold(),
            error_map.len()
        );
    }

    File::create(PROTO_FILE)?.write_all(final_str.as_bytes())?;
    println!(
        "{}",
        "✅ Finished translating names of some proto classes."
            .green()
            .bold()
    );

    Ok(())
}
