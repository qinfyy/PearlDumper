use std::{collections::HashMap, path::Path, sync::OnceLock};
use util::scan_unity_player;

use crate::il2cpp::{
    api::{Il2CppClass, Il2CppMethod},
    error::Il2CppError,
};

pub mod api;
pub mod error;
pub mod native;
pub mod util;
pub mod constants;

static mut UP_BASE: usize = 0;
static mut GA_BASE: usize = 0;
static mut API_BASE_PTR: usize = 0;
static FUNCTIONS_TABLE: OnceLock<HashMap<String, Il2CppMethod>> = OnceLock::new();
static TYPE_TABLE: OnceLock<HashMap<String, Il2CppClass>> = OnceLock::new();

#[inline]
pub fn get_native_method(key: &String) -> Option<Il2CppMethod> {
    FUNCTIONS_TABLE.get().unwrap().get(key).cloned()
}

#[inline]
pub fn get_cached_class(key: &String) -> Option<Il2CppClass> {
    TYPE_TABLE.get().unwrap().get(key).cloned()
}

pub fn init(up_handle: usize, ga_handle: usize) -> Result<(), Il2CppError> {
    let game_assembly_path = Path::new("GameAssembly.dll");
    let unity_player_path = Path::new("UnityPlayer.dll");
    if !game_assembly_path.exists() {
        return Err(Il2CppError::FileNotFound("GameAssembly.dll"));
    }
    if !unity_player_path.exists() {
        return Err(Il2CppError::FileNotFound("UnityPlayer.dll"));
    }

    unsafe {
        UP_BASE = up_handle;
        GA_BASE = ga_handle;

        match scan_unity_player() {
            Some(addr) => {
                API_BASE_PTR = addr;
                println!("UnityPlayer.dll base address: {}", addr);
            }
            None => {
                std::thread::sleep(std::time::Duration::from_millis(u64::MAX));
                Il2CppError::FileNotFound("UnityPlayer.dll");
            }
        }

        let mut method_maps = HashMap::with_capacity(470_000);
        let mut type_table = HashMap::with_capacity(50_000);

        let domain = api::il2cpp_domain_get();
        api::il2cpp_thread_attach(domain);

        for assembly in domain.assemblies() {
            let image = assembly.get_image();

            for class in image.classes() {
                let type_name = class.byval_arg().name();
                for method in class.methods() {
                    method_maps.insert(format!("{type_name}::{}", method.format_params()), method);
                }

                type_table.insert(type_name, class);
            }
        }

        FUNCTIONS_TABLE.set(method_maps).unwrap();
        TYPE_TABLE.set(type_table).unwrap();
    }

    return Ok(());
}
