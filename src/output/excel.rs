use std::{
    collections::HashMap,
    error::Error,
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    result::Result,
};

use crate::{
    core::{
        assembly::MonoAssembly,
        json_convert::JsonConvert,
        runtime::{MonoField, RuntimeType},
        string::SystemString,
    },
    il2cpp::{
        get_cached_class,
        native::{Il2CppArray, NativeArray, NativeDictionary},
    },
    output::util::{
        GREEN_PROGRESS_BAR, YELLOW_PROGRESS_BAR, create_progress_bar, get_runtime_type_name,
        report_errors,
    },
};

use colored::Colorize;
use lazy_static::lazy_static;
use std::sync::Mutex;

lazy_static! {
    static ref PLANES: Mutex<Vec<serde_json::Value>> = Mutex::new(Vec::new());
    static ref ADVENTURE_PLAYER: Mutex<Vec<serde_json::Value>> = Mutex::new(Vec::new());
    static ref SUMMON_UNIT_DATA: Mutex<Vec<serde_json::Value>> = Mutex::new(Vec::new());
    static ref PERFORMANCE_DS: Mutex<Vec<serde_json::Value>> = Mutex::new(Vec::new());
}

pub fn get_planes() -> Vec<serde_json::Value> {
    PLANES.lock().unwrap().clone()
}

pub fn get_adventure_player() -> Vec<serde_json::Value> {
    ADVENTURE_PLAYER.lock().unwrap().clone()
}

pub fn get_summon_unit_data() -> Vec<serde_json::Value> {
    SUMMON_UNIT_DATA.lock().unwrap().clone()
}

pub fn get_performance_ds() -> Vec<serde_json::Value> {
    PERFORMANCE_DS.lock().unwrap().clone()
}

fn check_fields_class(fields: &Il2CppArray) -> Result<(bool, Vec<String>), Box<dyn Error>> {
    if fields.len() >= 4 {
        let mut has_spath_field = false;
        let mut has_data_dict_field = false;
        // let mut has_hash_index_field = false;
        let mut has_is_data_loaded_field = false;
        let mut str_field: MonoField = MonoField::new(0);
        let mut dict_field: MonoField = MonoField::new(0);
        for i in 0..fields.len() {
            let field_ptr: &usize = fields.get(i);
            let field = MonoField::new(*field_ptr);
            if get_runtime_type_name(&field.get_field_type()?, false)? == "String[]" {
                has_spath_field = true;
                if field.get_name()?.as_str() != "fullPathList" {
                    str_field = field;
                }
            } else if get_runtime_type_name(&field.get_field_type()?, false)?
                .starts_with("Dictionary<")
            {
                has_data_dict_field = true;
                dict_field = field;
            // } else if get_runtime_type_name(&field.get_field_type()?, false)? == "Byte" {
            //     has_hash_index_field = true;
            } else if get_runtime_type_name(&field.get_field_type()?, false)? == "Boolean" {
                has_is_data_loaded_field = true;
            }
        }

        if has_spath_field
            && has_data_dict_field
            // && has_hash_index_field
            && has_is_data_loaded_field
            && str_field.0 != 0
        {
            unsafe {
                let _spath = str_field.get_value(0);
                let spath = match _spath {
                    Ok(path) => path,
                    Err(_) => {
                        return Ok((false, vec![]));
                    }
                };

                let arr_native = &*(spath.0 as *const NativeArray<SystemString>);
                let mut list_spath: Vec<String> = vec![];
                for j in 0..arr_native.max_length as usize {
                    let item = arr_native.vector[j];
                    list_spath.push(item.as_str().to_string());
                }
                return Ok((true, list_spath));
            }
        }
    }
    Ok((false, vec![]))
}

pub fn excel_dump(mono_assembly: &MonoAssembly) -> Result<(), Box<dyn Error>> {
    println!("{}", "📦 Scanning and filtering classes...".green().bold());
    let types = mono_assembly.get_types(60 != 0)?;
    let mut list_excel: HashMap<String, Vec<String>> = HashMap::new();

    let pb_filter = create_progress_bar(types.len() as u64, YELLOW_PROGRESS_BAR);

    for j in 0..types.len() {
        let runtime_ptr: &usize = types.get(j);
        let runtime_type = RuntimeType::new(*runtime_ptr);
        pb_filter.set_message(format!(
            "Scanning type {}",
            runtime_type.get_full_name()?.as_str()
        ));

        let fields = runtime_type.get_fields(60)?;
        let is_valid = check_fields_class(&fields)?;
        if is_valid.0 {
            list_excel.insert(
                runtime_type.get_full_name()?.as_str().to_string(),
                is_valid.1,
            );
        }
        pb_filter.inc(1);
    }

    pb_filter.finish_with_message("✅ Finished filtering types.");
    println!(
        "{}",
        "✅ Finished scanning and filtering classes..."
            .green()
            .bold()
    );

    println!("{}", "📦 Processing excel data...".green().bold());
    let len = list_excel.len();
    let pb = create_progress_bar(len as u64, GREEN_PROGRESS_BAR);
    let mut error_count = 0;
    for (key, values) in list_excel.iter() {
        pb.set_message(format!("Processing {}", key.clone()));

        unsafe {
            let class_cf = get_cached_class(&key).ok_or(anyhow::anyhow!("no such class"))?;

            let _mt_load_data = class_cf
                .find_method("LoadData", &[])
                .ok_or(anyhow::anyhow!("no such method"));

            let _ = match _mt_load_data {
                Ok(mt_load_data) => {
                    let func_ld: extern "fastcall" fn() -> usize =
                        std::mem::transmute(mt_load_data.va());

                    let _ = microseh::try_seh(|| func_ld()).map_err(|e| {
                        anyhow::anyhow!(
                            "Runtime Exception! (type: {} method: {}): {:?}",
                            class_cf.byval_arg().name(),
                            mt_load_data.name(),
                            e
                        )
                    });
                }
                Err(_) => {}
            };

            let mt_get_dict = class_cf
                .find_method_by_return_type("System.Collections.Generic.Dictionary<", &[])
                .ok_or(anyhow::anyhow!("no such method"))?;

            let func_gd: extern "fastcall" fn() -> usize = std::mem::transmute(mt_get_dict.va());

            let _dict_ptr = microseh::try_seh(|| func_gd()).map_err(|e| {
                anyhow::anyhow!(
                    "Runtime Exception! (type: {} method: {}): {:?}",
                    class_cf.byval_arg().name(),
                    mt_get_dict.name(),
                    e
                )
            });

            let dict_ptr = match _dict_ptr {
                Ok(dict_ptr) => dict_ptr,
                Err(_) => {
                    error_count += 1;
                    continue;
                }
            };
            
            let res_dict = &*(dict_ptr as *const NativeDictionary<usize, usize>);
            let mut list_json: Vec<serde_json::Value> = vec![];

            if let Some(entries) = res_dict.entries.as_ref() {
                for i in 0..res_dict.count as usize {
                    let entry = entries.vector[i];
                    if entry.hash_code >= 0 {
                        let json = JsonConvert::serialize_object(entry.value)?;
                        let value: serde_json::Value = serde_json::from_str(&json.as_str())?;
                        if values.contains(&String::from("MazePlane")) {
                            PLANES.lock().unwrap().push(value.clone());
                        } else if values.contains(&String::from("AdventurePlayer")) {
                            ADVENTURE_PLAYER.lock().unwrap().push(value.clone());
                        } else if values.contains(&String::from("SummonUnitData")) {
                            SUMMON_UNIT_DATA.lock().unwrap().push(value.clone());
                        } else if values.contains(&String::from("PerformanceDS")) {
                            PERFORMANCE_DS.lock().unwrap().push(value.clone());
                        }
                        list_json.push(value);
                    }
                }
            }
            let json_str = serde_json::to_string_pretty(&list_json)?;
            let output_dir = Path::new("ExcelOutput");

            for name in values {
                let rel_path = Path::new(&name);
                let full_path: PathBuf = output_dir.join(rel_path).with_extension("json");
                if let Some(parent) = full_path.parent() {
                    fs::create_dir_all(parent)?;
                }

                let mut file = File::create(full_path)?;
                file.write_all(json_str.as_bytes())?;
            }
        }

        pb.inc(1);
    }
    pb.finish_with_message("✅ Finished dumping all Excel data.");
    report_errors(error_count, "dumping Excel data");
    println!("{}", "✅ Finished dumping Excel data.".green().bold());
    Ok(())
}
