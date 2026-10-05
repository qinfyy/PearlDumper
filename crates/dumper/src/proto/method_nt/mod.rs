use crate::proto::get_cached_class;
use crate::proto::recovery::Hints;
use crate::proto::util;
use anyhow::Context;
use il2cpp::vm::{metadata_cache, value::Il2CppValue};
use reflection::method_info::MethodInfo;
use reflection::runtime_type::RuntimeType;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

pub static CACHED_NT_MAP: OnceLock<HashMap<String, String>> = OnceLock::new();

pub mod method;
mod param_nt;
mod param_table;

pub fn get_method_nt_map() -> anyhow::Result<(HashMap<String, Vec<String>>, Hints)> {
    log::info!("[Method NT] Recovering names from the parameter table");
    let (method_map, mut proto_name_map) =
        crate::guarded(|| Ok(param_nt::process_table(param_table::PARAM_NT_MAP)))
            .context("method name recovery: parameter table")?;

    let method_entries = method::get_method_nt_entries()?;
    let method_count = method_entries.len();
    proto_name_map.extend(method_entries);

    log::info!("[Method NT] Recovering enum names");
    let enum_map =
        crate::guarded(|| Ok(get_enum_names())).context("method name recovery: enum names")?;
    let enum_count = enum_map.len();
    proto_name_map.extend(enum_map);

    log::info!(
        "[Method NT] {} candidates, {method_count} method observations, {enum_count} enum observations",
        proto_name_map.len()
    );
    let _ = CACHED_NT_MAP.set(proto_name_map.unique());

    Ok((method_map, proto_name_map))
}

pub fn dump_global_field_map() -> Hints {
    let mut proto_props = HashSet::<String>::new();

    for i in unsafe { il2cpp::RPG_NETWORK_PROTO_START }..unsafe { il2cpp::RPG_NETWORK_PROTO_END } {
        let Ok(runtime_type) =
            RuntimeType::from_class(metadata_cache::get_typeinfo_from_typedefindex(i))
        else {
            continue;
        };
        for prop in runtime_type.get_properties(62) {
            proto_props.insert(prop.get_name().unwrap().as_str().to_string());
        }
    }

    let mut map = Hints::new();

    for i in 0..unsafe { il2cpp::MAX_TYPEDEFINDEX } {
        let Ok(runtime_type) =
            RuntimeType::from_class(metadata_cache::get_typeinfo_from_typedefindex(i))
        else {
            continue;
        };

        for prop in runtime_type.get_properties(62) {
            let prop_name = prop.get_name().unwrap().as_str();
            if !proto_props.contains(prop_name.as_ref()) {
                continue;
            }

            if let Ok(get_method) = prop.get_get_method(true)
                && !get_method.is_null()
                && let Some(name) = get_method
                    .get_name()
                    .unwrap()
                    .as_str()
                    .strip_prefix("get_")
                    .filter(|n| *n != prop_name.as_ref())
            {
                map.insert(prop_name.to_string(), name.to_string());
                continue;
            }

            if let Ok(set_method) = prop.get_set_method(true)
                && !set_method.is_null()
                && let Some(name) = set_method
                    .get_name()
                    .unwrap()
                    .as_str()
                    .strip_prefix("set_")
                    .filter(|n| *n != prop_name.as_ref())
            {
                map.insert(prop_name.to_string(), name.to_string());
            }
        }
    }
    map
}

pub fn get_enum_names() -> Hints {
    let mut output = Hints::new();

    let Some(class) = get_cached_class("XLua.ObjectTranslator.IniterAdderUnityEngineVector2")
    else {
        // This optional name source was removed in 4.6.51; enum values still come
        // from their own metadata and do not depend on the old XLua helper.
        log::info!("Legacy XLua enum-name helper is absent; retaining metadata enum names");
        return output;
    };
    for method in class.get_methods() {
        let m_name = method.get_name();

        if !m_name.starts_with("Proto") {
            continue;
        }

        let mi = MethodInfo::from_handle(method).unwrap();

        let args = mi.get_parameters();

        let Some(first_arg) = args.first() else {
            continue;
        };

        let obf_name = first_arg.get_parameter_type().unwrap().il_name();

        if !util::is_obf(&obf_name) {
            continue;
        }

        let Some(deobf_name) = m_name
            .strip_prefix("Proto")
            .and_then(|sp| sp.strip_suffix("_cast"))
        else {
            continue;
        };

        output.insert(obf_name.to_string(), deobf_name.to_string());
    }

    output
}
