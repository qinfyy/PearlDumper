use crate::proto::recovery::Hints;
use crate::proto::util;
use il2cpp::get_cached_class;
use il2cpp::vm::value::Il2CppValue;
use reflection::{method_info::MethodInfo, runtime_type::RuntimeType};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy)]
pub enum ExtractMode {
    FirstParam,
    FirstParamBeforeUInt32,
    SecondParam,
    #[allow(dead_code)]
    ThirdParam,
    ReturnType,
    ReturnTypeNoArgsInHierarchy,
}

fn get_method(
    class_name: &str,
    method_name: &str,
    mode: ExtractMode,
) -> Option<il2cpp::vm::method::Il2CppMethod> {
    let runtime_type = RuntimeType::from_class(get_cached_class(class_name)?).ok()?;
    if matches!(mode, ExtractMode::ReturnTypeNoArgsInHierarchy) {
        let mut owner = runtime_type;
        let mut seen = HashSet::new();
        // Only this rule opts into declared-method lookup up the base chain.
        // A shadowing declaration or overload ambiguity must not fall through.
        while !owner.is_null() && seen.insert(owner) && seen.len() <= 64 {
            let matches: Vec<_> = owner
                .get_methods_il2cpp()
                .into_iter()
                .filter(|m| m.get_name().is_ok_and(|n| n.as_str() == method_name))
                .collect();
            if !matches.is_empty() {
                let [method] = matches.as_slice() else {
                    return None;
                };
                return (!method.is_static()
                    && method.get_parameters().is_empty()
                    && method
                        .get_return_type()
                        .is_ok_and(|t| util::is_obf(&t.il_name())))
                .then(|| method.get_il2cpp_method());
            }
            owner = owner.get_base_type().ok()?;
        }
        return None;
    }
    if matches!(mode, ExtractMode::FirstParamBeforeUInt32) {
        let matches: Vec<_> = runtime_type
            .get_methods_il2cpp()
            .into_iter()
            .filter(|m| {
                if m.is_static() || !m.get_name().is_ok_and(|n| n.as_str() == method_name) {
                    return false;
                }
                let params = m.get_parameters();
                let [first, second] = params.as_slice() else {
                    return false;
                };
                first
                    .get_parameter_type()
                    .is_ok_and(|t| util::is_obf(&t.il_name()))
                    && second
                        .get_parameter_type()
                        .is_ok_and(|t| t.il_name() == "System.UInt32")
            })
            .collect();
        let [method] = matches.as_slice() else {
            return None;
        };
        return Some(method.get_il2cpp_method());
    }
    let method_info = runtime_type.get_method(method_name.into(), 62).ok()?;
    if method_info.0 == 0 {
        None
    } else {
        Some(method_info.get_il2cpp_method())
    }
}

fn get_param_type(method_info: &MethodInfo, mode: ExtractMode) -> Option<RuntimeType> {
    let params = method_info.get_parameters();
    match mode {
        ExtractMode::FirstParam | ExtractMode::FirstParamBeforeUInt32 => {
            params.first()?.get_parameter_type().ok()
        }
        ExtractMode::SecondParam => params.get(1)?.get_parameter_type().ok(),
        ExtractMode::ThirdParam => params.get(2)?.get_parameter_type().ok(),
        ExtractMode::ReturnType | ExtractMode::ReturnTypeNoArgsInHierarchy => {
            method_info.get_return_type().ok()
        }
    }
}

fn clean_type_name(runtime_type: RuntimeType, il_name: &str) -> String {
    let generics = runtime_type.get_generic_arguments();

    if !generics.is_empty() {
        generics
            .last()
            .map_or_else(|| il_name.to_string(), |t| t.il_name().into_owned())
    } else if il_name.ends_with("[]") {
        runtime_type.get_element_type().ok().map_or_else(
            || il_name.trim_end_matches("[]").to_string(),
            |t| t.il_name().into_owned(),
        )
    } else {
        il_name.to_string()
    }
}

pub fn process_table(
    table: &[(&str, &str, &str, ExtractMode)],
) -> (HashMap<String, Vec<String>>, Hints) {
    let mut method_map: HashMap<String, Vec<String>> = HashMap::new();
    let mut proto_name_map = Hints::new();

    for (class_name, method_name, output_key, mode) in table {
        let Some(native_method) = get_method(class_name, method_name, *mode) else {
            log::debug!("[Param NT] method not found: {class_name}::{method_name}");
            continue;
        };
        let rva = native_method.rva();
        let owner = RuntimeType::from_class(native_method.class())
            .map(|ty| ty.il_name().into_owned())
            .unwrap_or_else(|_| class_name.to_string());
        proto_name_map.context(format!("{owner}::{method_name}"), rva);

        if let Ok(method_info) = MethodInfo::from_handle(native_method)
            && let Some(param_type) = get_param_type(&method_info, *mode)
        {
            let il_name = param_type.il_name();
            let clean = clean_type_name(param_type, &il_name);
            if clean.len() == 11 && clean.chars().all(char::is_uppercase) {
                if rva != 0 {
                    method_map
                        .entry(clean.clone())
                        .or_default()
                        .push(format!("0x{rva:X}"));
                }
                proto_name_map.insert(clean, output_key.to_string());
            } else if clean.rsplit('.').next() == Some(*output_key) {
                log::debug!("[Param NT] already readable: {owner}::{method_name} -> {clean}");
            } else {
                log::debug!(
                    "[Param NT] param name mismatch: {class_name}::{method_name} -> {output_key}, got \"{clean}\""
                );
            }
        }
    }

    (method_map, proto_name_map)
}
