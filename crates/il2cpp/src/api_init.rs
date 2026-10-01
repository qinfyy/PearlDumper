use indexmap::IndexMap;
use std::time::{Duration, Instant};

use crate::{
    CLASS_TABLE, CLASS_TABLE_VEC, FUNCTIONS_TABLE, api,
    vm::{thread::Il2CppThread, r#type::Il2CppTypeNameFormat},
};

pub fn init_il2cpp(debug: bool) -> anyhow::Result<()> {
    if CLASS_TABLE.get().is_some() {
        return Ok(());
    }
    let started = Instant::now();
    crate::profile::validate_loaded_client()?;
    let domain = api::wait_until_ready(Duration::from_secs(120))?;
    unsafe {
        log::info!("Attaching dump worker to IL2CPP domain 0x{:X}", domain.0);
        let thread = Il2CppThread::attach(domain);
        anyhow::ensure!(thread.0 != 0, "failed to attach dump worker to IL2CPP");
        log::info!("Dump worker attached: IL2CPP thread=0x{:X}", thread.0);

        let mut method_maps = IndexMap::with_capacity(700_000);
        let mut type_map = IndexMap::with_capacity(81_000);
        let mut type_vec = Vec::with_capacity(81_000);

        let mut assembly_count = 0;
        log::info!("Reading assembly list for type and method scan");
        let assembly_array = api::il2cpp_domain_get_assemblies(domain, &mut assembly_count);
        anyhow::ensure!(
            (1..=1024).contains(&assembly_count),
            "invalid assembly count {assembly_count}"
        );
        anyhow::ensure!(
            utils::memory::is_readable(
                assembly_array as usize,
                assembly_count * size_of::<usize>()
            ),
            "invalid assembly array"
        );
        log::info!("Scanning {assembly_count} assemblies");

        let mut type_def_index = 0;
        let mut proto_start = 0;
        let mut proto_end = 0;
        let mut csharp_start = 0;
        for i in 0..assembly_count {
            let assembly_started = Instant::now();
            let mut last_progress = assembly_started;
            let assembly = *assembly_array.wrapping_add(i);
            log::debug!(
                "Reading assembly {}/{assembly_count} at 0x{:X}",
                i + 1,
                assembly.0
            );
            let image = api::il2cpp_assembly_get_image(assembly);
            let module = image.get_name();
            let class_count = image.get_count();

            anyhow::ensure!(class_count < 1_000_000, "invalid class count in {module}");
            log::info!(
                "Scanning assembly {}/{assembly_count}: {module}, {class_count} classes",
                i + 1
            );

            if module == "RPG.Network.Proto.dll" {
                proto_start = type_def_index;
            } else if module == "Assembly-CSharp.dll" {
                csharp_start = type_def_index;
            }

            for class_index in 0..class_count {
                if class_index == 0 || last_progress.elapsed() >= Duration::from_secs(2) {
                    log::info!(
                        "Scanning {module}: {class_index}/{class_count} classes, total {type_def_index} types / {} methods, {:.1}s elapsed",
                        method_maps.len(),
                        started.elapsed().as_secs_f64()
                    );
                    last_progress = Instant::now();
                }
                let class = api::il2cpp_image_get_class(image, class_index as usize);
                anyhow::ensure!(class.0 != 0, "null class in {module}, index {class_index}");

                if debug {
                    log::debug!("{type_def_index} 0x{:X}", class.0);
                }

                let type_name = class.byval_arg().get_name(Il2CppTypeNameFormat::IL);
                let methods = class.get_methods();
                for (i, method) in methods.iter().enumerate() {
                    let params = method.signature(i);
                    method_maps.insert(format!("{type_name}::{params}"), *method);
                }
                type_map.insert(type_name, class);
                type_vec.push(class);
                type_def_index += 1;
            }
            if module == "RPG.Network.Proto.dll" {
                proto_end = type_def_index;
            }
            log::info!(
                "Finished {module}: {class_count} classes in {:.2}s",
                assembly_started.elapsed().as_secs_f64()
            );
        }

        log::info!(
            "IL2CPP scan complete: {} methods and {} types in {:.2}s",
            method_maps.len(),
            type_def_index,
            started.elapsed().as_secs_f64()
        );
        anyhow::ensure!(
            type_map.contains_key("System.Object") && type_map.contains_key("System.RuntimeType"),
            "core runtime classes missing; client layout is not supported"
        );
        crate::MAX_TYPEDEFINDEX = type_def_index;
        crate::RPG_NETWORK_PROTO_START = proto_start;
        crate::RPG_NETWORK_PROTO_END = proto_end;
        crate::ASSEMBLY_CSHARP_START = csharp_start;
        FUNCTIONS_TABLE.set(method_maps).unwrap();
        CLASS_TABLE.set(type_map).unwrap();
        CLASS_TABLE_VEC.set(type_vec).unwrap();
        dump_methods_json()?;
    }
    log::info!(
        "IL2CPP initialization complete in {:.2}s",
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

pub fn dump_methods_json() -> anyhow::Result<()> {
    crate::method_dump::write("DUMP/methods.json", FUNCTIONS_TABLE.get().unwrap())
}
