use std::{
    collections::HashMap,
    error::Error,
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    result::Result, thread, time::Duration,
};

use crate::{
    core::{
        assembly::MonoAssembly, excel_table::ExcelTable, json_convert::JsonConvert, runtime::{MonoField, RuntimeType}, string::SystemString
    },
    il2cpp::{
        native::{Il2CppArray, NativeArray, NativeDictionary}
    },
    output::util::{
        GREEN_PROGRESS_BAR, YELLOW_PROGRESS_BAR, create_progress_bar, report_errors
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

#[derive(Debug, Clone)]
pub struct ExcelData {
    pub spath: Vec<String>,
}

pub fn check_fields_class(fields: &Il2CppArray) -> Result<(bool, ExcelData), Box<dyn Error>> {
    if fields.len() >= 4 {

        let mut str_field: MonoField = MonoField::new(0);
        for i in 0..fields.len() {
            let field = fields.get::<MonoField>(i);
            let field_name = field.get_name()?.as_str();
            if field_name == "s_PathList" || field_name == "MEAAIFJAEFK" { //4.0.0
                str_field = field.clone();
            }
        }

        if str_field.0 != 0 {
            unsafe {
                let ptr: *const std::ffi::c_void = std::ptr::null();
                let _spath = str_field.get_value(ptr as usize);
                let spath = match _spath {
                    Ok(path) => path,
                    Err(_) => {
                        return Ok((false, ExcelData {
                            spath: vec![],
                        }));
                    }
                };

                let arr_native = &*(spath.0 as *const NativeArray<SystemString>);
                let mut list_spath: Vec<String> = vec![];
                for j in 0..arr_native.max_length as usize {
                    let item = arr_native.vector[j];
                    list_spath.push(item.as_str().to_string());
                }
                return Ok((true, ExcelData {
                    spath: list_spath,
                }));
            }
        }
    }
    Ok((false, ExcelData {
        spath: vec![],
    }))
}


pub fn excel_dump(mono_assembly: &MonoAssembly) -> Result<(), Box<dyn Error>> {
        println!("\r");

        for i in (1..=60).rev() {
            print!("\r⏳ Waiting {:2} seconds before dump excel...   ", i); 
            std::io::Write::flush(&mut std::io::stdout()).unwrap();
            thread::sleep(Duration::from_secs(1));
        }

        println!("\r✅ Done!                    ");


    println!("{}", "📦 Scanning and filtering classes...".green().bold());
    let types = mono_assembly.get_types(60 != 0)?;
    let mut list_excel: HashMap<String, ExcelData> = HashMap::new();

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
            let excel_table = ExcelTable::new(key.as_str());
            excel_table.cctor()?;
            if !excel_table.is_data_loaded()? || excel_table.is_data_empty()? {
                //println!("load data {}", key);
                match excel_table.load_data() {
                    Err(_) => {
                        eprintln!("Failed to load_data {}", key);
                        error_count += 1;
                        continue;
                    }
                    _ => {}
                }
            }


            let dict_ptr = match excel_table.get_excel_table() {
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
                        //let json = JsonConvert::serialize_object(entry.value).expect("json serialize error");
                        if !key.contains("ConstValueClientExcelTable") {
                            match JsonConvert::serialize_object(entry.value) {
                                Ok(json) => {
                                    // 成功序列化时继续处理
                                    let value: serde_json::Value = serde_json::from_str(&json.as_str())?;
                                    list_json.push(value);
                                }
                                Err(_) => {
                                    // 序列化失败时，可以选择继续
                                    eprintln!("Failed to serialize {} entry value at index {}", key, i);
                                    error_count += 1;
                                    // 或者可以选择跳过这个条目，继续执行
                                    continue;
                                }
                            }
                        }
                        //let value: serde_json::Value = serde_json::from_str(&json.as_str())?;
                        // if values.spath.contains(&String::from("MazePlane")) {
                        //     PLANES.lock().unwrap().push(value.clone());
                        // } else if values.spath.contains(&String::from("AdventurePlayer")) {
                        //     ADVENTURE_PLAYER.lock().unwrap().push(value.clone());
                        // } else if values.spath.contains(&String::from("SummonUnitData")) {
                        //     SUMMON_UNIT_DATA.lock().unwrap().push(value.clone());
                        // } else if values.spath.contains(&String::from("PerformanceDS")) {
                        //     PERFORMANCE_DS.lock().unwrap().push(value.clone());
                        // }
                        //list_json.push(value);
                    }
                }
            }
            let json_str = serde_json::to_string_pretty(&list_json)?;
            let output_dir = Path::new("ExcelOutput");

            for name in values.spath.clone() {
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
