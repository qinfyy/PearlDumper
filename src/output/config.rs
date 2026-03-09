use std::{
    error::Error,
    fs::{self, File},
    io::Write,
    path::Path,
};

use colored::Colorize;

use crate::{
    core::{
        game_config::GameCoreConfigLoader, json_convert::JsonConvert, json_serializer::JsonSerializer, object::SystemObject, string::SystemString, writer::{JsonTextWriter, StreamWriter, SystemDirectory}
    },
    output::{excel::{get_adventure_player, get_performance_ds, get_planes, get_summon_unit_data}, util::{create_progress_bar, report_errors, search_and_replace_dyn_float, setup_json_serializer, GREEN_PROGRESS_BAR}},
};


pub fn config_dump() -> Result<(), Box<dyn Error>> {
    println!("{}", "[*] Processing Summon Unit...".green().bold());
    summon_unit_dump()?;
    println!("{}", "[+] Finished processing summon unit...".green().bold());

    println!("{}", "[*] Processing Adventure Player...".green().bold());
    adventure_player_dump()?;
    println!("{}", "[+] Finished processing adventure player...".green().bold());

    println!("{}", "[*] Processing Map...".green().bold());
    map_dump()?;
    println!("{}", "[+] Finished processing map...".green().bold());

    println!("{}", "[*] Processing Floor...".green().bold());
    let group_map = floor_dump()?;
    println!("{}", "[+] Finished processing floor...".green().bold());

    println!("{}", "[*] Processing Group...".green().bold());
    group_dump(group_map)?;
    println!("{}", "[+] Finished processing group...".green().bold());

    // println!("{}", "[*] Processing Performance DS...".green().bold());
    // performance_ds_dump()?;
    // println!("{}", "[+] Finished processing performance ds...".green().bold());
    Ok(())
}

pub fn floor_dump() -> Result<Vec<String>, Box<dyn Error>> {
    let planes = get_planes();
    let mut planes_len = 0;
    for i in 0..planes.len() {
        let plane = planes.get(i).unwrap();
        planes_len += plane["FloorIDList"].as_array().unwrap().len();
    }

    let mut group_map: Vec<String> = vec![];

    let pb_filter = create_progress_bar(planes_len as u64, GREEN_PROGRESS_BAR);
    let mut error_count: i32 = 0;
    for i in 0..planes.len() {
        let plane = planes.get(i).unwrap();
        let plane_id = plane["PlaneID"].as_u64().unwrap();
        pb_filter.set_message(format!("Scanning plane: {}", plane_id));
        for j in 0..plane["FloorIDList"].as_array().unwrap().len() {
            pb_filter.inc(1);
            let floor_id = plane["FloorIDList"].as_array().unwrap().get(j).unwrap();
            let group_path = format!(
                "Config/LevelOutput/RuntimeFloor/P{}_F{}.json",
                plane_id, floor_id
            );
            let path_value = group_path.replace('\r', "");
            let out_file = path_value.clone();
            let _group_ptr = GameCoreConfigLoader::load_rt_level_floor_info(SystemString::from(out_file.clone()));
            let group_ptr = match _group_ptr {
                Ok(group_ptr) => group_ptr,
                Err(_) => {
                    error_count += 1;
                    continue;
                }
            };
            let json_raw = JsonConvert::serialize_object(group_ptr.0)?;
            let json_value: serde_json::Value = serde_json::from_str(&json_raw.as_str())?;
            let json_str = serde_json::to_string_pretty(&json_value)?;
            let full_path = Path::new(&out_file);
            if let Some(parent) = full_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut file = File::create(full_path)?;
            file.write_all(json_str.as_bytes())?;

            if let Some(group_list) = json_value["GroupInstanceList"].as_array() {
                for group_obj in group_list {
                    if let Some(group_path) = group_obj.get("GroupPath").and_then(|v| v.as_str()) {
                        group_map.push(group_path.to_string());
                    }
                }
            }
        }
    }
    pb_filter.finish_with_message("[+] Finished dumping runtime floor.");
    report_errors(error_count, "dumping runtime floor");
    Ok(group_map)
}

pub fn map_dump() -> Result<(), Box<dyn Error>> {
    let planes = get_planes();
    let mut planes_len = 0;
    for i in 0..planes.len() {
        let plane = planes.get(i).unwrap();
        planes_len += plane["FloorIDList"].as_array().unwrap().len();
    }

    let pb_filter = create_progress_bar(planes_len as u64, GREEN_PROGRESS_BAR);
    let json_serializer = setup_json_serializer(1, 1, 1, 1, 4)?;

    let mut error_count = 0;

    for i in 0..planes.len() {
        let plane = planes.get(i).unwrap();
        let plane_id = plane["PlaneID"].as_u64().unwrap();
        pb_filter.set_message(format!("Scanning plane: {}", plane_id));
        for j in 0..plane["FloorIDList"].as_array().unwrap().len() {
            pb_filter.inc(1);
            let floor_id = plane["FloorIDList"].as_array().unwrap().get(j).unwrap();
            let group_path = format!(
                "Config/LevelOutput/Map/MapInfo_P{}_F{}.json",
                plane_id, floor_id
            );
            let path_value = group_path.replace('\r', "");
            let out_file = path_value.clone();
            let values: Vec<&str> = path_value.split('/').collect();
            let filename = values.last().unwrap();
            let out_folder = out_file.strip_suffix(filename).unwrap_or("").to_string();
            let _group_ptr = GameCoreConfigLoader::load_level_navmap_config(SystemString::from(
                out_file.clone(),
            ));
            let group_ptr= match _group_ptr {
                Ok(group_ptr) => group_ptr,
                Err(_) => {
                    continue;
                }
            };
            let _ = search_and_replace_dyn_float(SystemObject::new(group_ptr.0));
            process_file(&json_serializer, &out_file, &out_folder, group_ptr, &mut error_count)?;
        }
    }
    pb_filter.finish_with_message("[+] Finished dumping navmap.");
    report_errors(error_count, "dumping navmap");
    Ok(())
}


pub fn summon_unit_dump() -> Result<(), Box<dyn Error>> {
    let summon_unit_data = get_summon_unit_data();
    let pb_filter = create_progress_bar(summon_unit_data.len() as u64, GREEN_PROGRESS_BAR);
    let json_serializer = setup_json_serializer(1, 1, 0, 1, 4)?;

    let mut error_count: i32 = 0;

    for i in 0..summon_unit_data.len() {
        let summon_unit = summon_unit_data.get(i).unwrap();
        let group_path = summon_unit["JsonPath"].as_str().unwrap();
        pb_filter.set_message(format!("Scanning summon unit path: {}", group_path));
        let path_value = group_path.replace('\r', "");
        let out_file = path_value.clone();
        let values: Vec<&str> = path_value.split('/').collect();
        let filename = values.last().unwrap();
        let out_folder = out_file.strip_suffix(filename).unwrap_or("").to_string(); 
        let _group_ptr = GameCoreConfigLoader::load_summon_unit_config(SystemString::from(group_path));
        let group_ptr = match _group_ptr {
            Ok(group_ptr) => group_ptr,
            Err(_) => {
                error_count += 1;
                continue;
            }
        };
        process_file(&json_serializer, &out_file, &out_folder, group_ptr, &mut error_count)?;
        pb_filter.inc(1);
    }
    pb_filter.finish_with_message("[+] Finished dumping summon unit.");
    report_errors(error_count, "dumping summon unit");
    Ok(())
}

pub fn adventure_player_dump() -> Result<(), Box<dyn Error>> {
    let adventure_player_data = get_adventure_player();
    let pb_filter = create_progress_bar(adventure_player_data.len() as u64, GREEN_PROGRESS_BAR);
    let json_serializer = setup_json_serializer(1, 1, 1, 1, 4)?;
    let mut error_count = 0;

    for i in 0..adventure_player_data.len() {
        let adventure_player = adventure_player_data.get(i).unwrap();
        let group_path = adventure_player["PlayerJsonPath"].as_str().unwrap();
        pb_filter.set_message(format!("Scanning adventure player path: {}", group_path));
        let path_value = group_path.replace('\r', "");
        let out_file = path_value.clone();
        let values: Vec<&str> = path_value.split('/').collect();
        let filename = values.last().unwrap();
        let out_folder = out_file.strip_suffix(filename).unwrap_or("").to_string();

        let _group_ptr = GameCoreConfigLoader::load_adventure_character_config(SystemString::from(group_path));
        let group_ptr = match _group_ptr {
            Ok(group_ptr) => group_ptr,
            Err(_) => {
                error_count += 1;
                continue;
            }
        };

        let _ = search_and_replace_dyn_float(SystemObject::new(group_ptr.0));
        process_file(&json_serializer, &out_file, &out_folder, group_ptr, &mut error_count)?;
        pb_filter.inc(1);
    }
    pb_filter.finish_with_message("[+] Finished dumping adventure player.");
    report_errors(error_count, "dumping adventure player");
    Ok(())
}

pub fn performance_ds_dump() -> Result<(), Box<dyn Error>> {
    let performance_ds_data = get_performance_ds();
    let pb_filter = create_progress_bar(performance_ds_data.len() as u64, GREEN_PROGRESS_BAR);
    let json_serializer = setup_json_serializer(1, 1, 1, 1, 0)?;
    let mut error_count = 0;

    for i in 0..performance_ds_data.len() {

        let performance_ds = performance_ds_data.get(i).unwrap();
        let group_path = performance_ds["PerformancePath"].as_str().unwrap();
        pb_filter.set_message(format!("Scanning performance ds path: {}", group_path));
        
        let path_value = group_path.replace('\r', "");
        let out_file = path_value.clone();
        let values: Vec<&str> = path_value.split('/').collect();
        let filename = values.last().unwrap();
        let out_folder = out_file.strip_suffix(filename).unwrap_or("").to_string();
        let _group_ptr =
            GameCoreConfigLoader::load_level_graph_config(SystemString::from(out_file.clone()));

        let group_ptr= match _group_ptr {
            Ok(group_ptr) => group_ptr,
            Err(_) => {
                error_count += 1;
                continue;
            }
        };
        let _ = search_and_replace_dyn_float(SystemObject::new(group_ptr.0));
        process_file(&json_serializer, &out_file, &out_folder, group_ptr, &mut error_count)?;
        pb_filter.inc(1);
    }
    pb_filter.finish_with_message("[+] Finished dumping performance ds.");

    report_errors(error_count, "dumping performance ds");
    Ok(())
}

pub fn group_dump(group_map: Vec<String>) -> Result<(), Box<dyn Error>> {
    let pb_filter = create_progress_bar(group_map.len() as u64, GREEN_PROGRESS_BAR);
    let json_serializer = setup_json_serializer(1, 1, 1, 1, 0)?;

    let mut error_count = 0;
    for group_path in group_map {
        pb_filter.set_message(format!("Scanning group: {}", group_path));
        let path_value = group_path.replace('\r', "");
        let out_file = path_value.clone();
        let values: Vec<&str> = path_value.split('/').collect();
        let filename = values.last().unwrap();
        let out_folder = out_file.strip_suffix(filename).unwrap_or("").to_string();
        let group_ptr =
            GameCoreConfigLoader::load_rt_level_group_info(SystemString::from(out_file.clone()))?;

        process_file(&json_serializer, &out_file, &out_folder, group_ptr, &mut error_count)?;
        pb_filter.inc(1);
    }
    pb_filter.finish_with_message("[+] Finished dumping runtime group.");
    report_errors(error_count, "dumping runtime group");
    Ok(())
}

pub fn process_file(json_serializer: &JsonSerializer, out_file: &String, out_folder: &String, group_ptr: SystemObject, error_count: &mut i32) -> Result<(), Box<dyn Error>> {
    let is_exists = SystemDirectory::exists(SystemString::from(out_folder.clone()));
    match is_exists {
        Ok(exists) => {
            if !exists {
                match SystemDirectory::create_directory(SystemString::from(out_folder.clone()))
                {
                    Ok(_) => {}
                    Err(_) => {
                        *error_count += 1;
                        return Ok(());
                    }
                }
            }
        }
        Err(_) => {
            *error_count += 1;
            return Ok(());
        }
    }
    let stream_writer = StreamWriter::new();
    stream_writer.constructor(SystemString::from(out_file.clone()))?;
    let json_text_writer = JsonTextWriter::new();
    match json_text_writer.constructor(stream_writer) {
        Ok(_) => {}
        Err(_) => {
            *error_count += 1;
            return Ok(());
        }
    }

    match json_serializer.serialize(json_text_writer, group_ptr.0) {
        Ok(_) => {}
        Err(_) => {
            *error_count += 1;
            return Ok(());
        }
    }
    match json_text_writer.close() {
        Ok(_) => {}
        Err(_) => {
            *error_count += 1;
            return Ok(());
        }
    }
    match stream_writer.close() {
        Ok(_) => {}
        Err(_) => {
            *error_count += 1;
            return Ok(());
        }
    }
    Ok(())
}