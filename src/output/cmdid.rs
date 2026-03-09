use colored::Colorize;

use crate::{
    core::{
        assembly::MonoAssembly,
        base_module::BaseModule,
        module_manager::ModuleManager,
        rsp_handler::RspHandler,
        runtime::{MonoField, RuntimeType},
    },
    il2cpp::{
        api::Il2CppMethod,
        native::{Il2CppObject, NativeArray, NativeDictionary, NativeList},
    }, output::util::{YELLOW_PROGRESS_BAR, create_progress_bar},
};
use std::{collections::HashMap, error::Error, fs::File, io::Write};

use lazy_static::lazy_static;
use std::sync::Mutex;

lazy_static! {
    static ref CMDID: Mutex<HashMap<String, usize>> = Mutex::new(HashMap::new());
}


pub fn get_cmdid() -> HashMap<String, usize> {
    CMDID.lock().unwrap().clone()
}


fn handler_add_packet(mono_assembly: &MonoAssembly) -> Result<(), Box<dyn Error>> {
    println!("{}", "[*] Add packet handlers...".green().bold());
   
    let types = mono_assembly.get_types(60 != 0)?;
    for i in 0..types.len() {
        let runtime_ptr: &usize = types.get(i);
        let runtime_type = RuntimeType::new(*runtime_ptr);
        if runtime_type.get_is_interface()? {
            continue;
        }
        if runtime_type.get_name()?.as_str() == "GlobalVars" {
            let fields = runtime_type.get_fields(60)?;
            for j in 0..fields.len() {
                let field = fields.get::<MonoField>(j);

                if field.get_name()?.as_str() == "s_ModuleManager" {
                    let module_manager_obj = ModuleManager::new(field.get_value(0)?.0);
                    let list_module = module_manager_obj.modules()?.to_vec::<BaseModule>();
                    
                    let pb_filter = create_progress_bar(list_module.len() as u64, YELLOW_PROGRESS_BAR);
                    for k in 0..list_module.len() {
                        let module = list_module.get(k);
                        if let Some(module) = module {
                            let child_add_ok = module.add_packet_handlers_child();
                            if child_add_ok.is_err() {
                                let parent_add_ok = module.add_packet_handlers_parent();
                                if parent_add_ok.is_err() {
                                    println!(
                                        "[-] Failed to add packet handlers {}",
                                        Il2CppObject::new(module.0).get_class().name().as_str()
                                    );
                                }
                            }
                        }
                        pb_filter.inc(1);
                    }
                    pb_filter.finish_with_message("[+] Finished add packet handlers");
                }
            }
        }
    }
    println!(
        "{}",
        "[+] Finished add packet handlers..."
            .green()
            .bold()
    );
    Ok(())
}

fn normalize_name(name: &str) -> String {
    if name.starts_with("_OnCmd") {
        return format!("Cmd{}", &name[6..]);
    }

    if name.starts_with("_On") {
        return format!("Cmd{}", &name[3..]);
    }
    if name.starts_with("_Cmd") {
        return format!("Cmd{}", &name[4..]);
    }
    if name.starts_with("Cmd") {
        return name.to_string();
    }

    name.to_string()
}

fn handler_dump_resp_and_notify(mono_assembly: &MonoAssembly) -> Result<(), Box<dyn Error>> {
    println!("{}", "[*] Dump response and notify...".green().bold());
    let types = mono_assembly.get_types(60 != 0)?;
    for i in 0..types.len() {
        let runtime_ptr: &usize = types.get(i);
        let runtime_type = RuntimeType::new(*runtime_ptr);
        if runtime_type.get_is_interface()? {
            continue;
        }

        if runtime_type.get_name()?.as_str() == "NotifyManager" {
            let fields = runtime_type.get_fields(60)?;
            for j in 0..fields.len() {
                let field = fields.get::<MonoField>(j);

                if field.get_name()?.as_str() == "_RspHandlers" {
                    let arr_obj = field.get_value(0)?;
                    unsafe {
                        let res_array = &*(arr_obj.0 as *const NativeArray<usize>);

                        for k in 0..res_array.max_length as usize {
                            let dict_ptr = res_array.vector[k];
                            let res_dict = &*(dict_ptr as *const NativeDictionary<usize, usize>);
                            if let Some(entries) = res_dict.entries.as_ref() {
                                let pb_filter = create_progress_bar(res_dict.count as u64, YELLOW_PROGRESS_BAR);
                                for l in 0..res_dict.count as usize {
                                    let entry = entries.vector[l];
                                    let list_handler =
                                        NativeList::new(entry.value).to_vec::<RspHandler>();
                                    let mut selected: Option<String> = None;
                                    let mut fist_rsp_name: Option<String> = None;
                                    for m in list_handler {
                                        let ptr = (m.0 as *const u8).add(0x28);
                                        let method_addr = *(ptr as *const usize);
                                        let method = Il2CppMethod::new(method_addr as usize);
                                        let raw_name = method.name();
                                        let normalized = normalize_name(raw_name.as_str());

                                        if normalized.starts_with("Cmd")
                                            && !raw_name.chars().all(|c| c.is_ascii_uppercase())
                                        {
                                            selected = Some(normalized);
                                            break;
                                        }

                                        if fist_rsp_name.is_none() {
                                            fist_rsp_name = Some(normalized);
                                        }
                                    }
                                    let final_name = selected.unwrap_or_else(|| {
                                        if let Some(first) = fist_rsp_name {
                                            if !first.starts_with("Cmd") {
                                                format!("Cmd{}", first)
                                            } else {
                                                first
                                            }
                                        } else {
                                            "CmdUnknown".to_string()
                                        }
                                    });
                                    CMDID.lock().unwrap().insert(final_name.clone(), entry.key);
                                    pb_filter.inc(1);
                                }
                                pb_filter.finish_with_message("[+] Finished dump response and notify");
                            }
                        }
                        let json = serde_json::to_string_pretty(&CMDID.lock().unwrap().clone()).unwrap();

                        let mut file = File::create("cmdid.json").unwrap();
                        file.write_all(json.as_bytes()).unwrap()

                    }
                }
            }
        }
    }
    println!("{}", "[+] Finished dump response and notify...".green().bold());

    Ok(())
}

pub fn cmdid_dump(mono_assembly: &MonoAssembly) -> Result<(), Box<dyn Error>> {
    handler_add_packet(mono_assembly)?;
    handler_dump_resp_and_notify(mono_assembly)?;
    Ok(())
}
