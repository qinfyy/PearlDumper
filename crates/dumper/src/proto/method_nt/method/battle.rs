use std::collections::HashMap;

use il2cpp::vm::value::Il2CppValue;
use reflection::{attributes::MethodAttributes, runtime_type::RuntimeType};

pub fn get_battle_send_nt() -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mapping = [
        ("<SendPVEBattleResultCsReq", "BattleOp"),
        ("<SendIdleLiveReplaceTeamCsReq", "KVP"),
    ];

    let Some(c_class) = il2cpp::get_cached_class("RPG.Client.NetworkManager.<>c") else {
        return map;
    };

    for method in c_class.get_methods() {
        let name = method.get_name();
        let Some(deobf) = mapping
            .iter()
            .find_map(|(prefix, expected)| name.starts_with(prefix).then_some(*expected))
        else {
            continue;
        };
        // .cctor/.ctor are first in this class's native list. Filter before
        // asking for a return type, and use the validated IL2CPP API directly.
        let return_type = il2cpp::api::il2cpp_method_get_return_type(method);
        if return_type.is_null() {
            log::warn!("[Method NT] {name} has no native return type");
            continue;
        }
        let obf_name = return_type.il_name();
        if obf_name.len() == 11 && obf_name.chars().all(char::is_uppercase) {
            log::debug!("[Method NT] param nt: {obf_name} -> {deobf}");
            map.insert(obf_name.into_owned(), deobf.to_string());
        }
    }

    if map.is_empty() {
        log::debug!("[Method NT] failed to find battle send nt");
    }
    map
}

pub fn get_compare_repeated_nt() -> HashMap<String, String> {
    let mut map = HashMap::new();
    const PREFIX: &str = "<_CompareRepeated";

    let handler = il2cpp::FUNCTIONS_TABLE
        .get()
        .expect("missing method cache")
        .iter()
        .find_map(|(signature, method)| {
            if !signature.contains("_CompareRepeatedGridFightNPCInfo") {
                return None;
            }
            let class = method.class();
            if class.get_image().get_name() != "Assembly-CSharp.dll"
                || !class
                    .get_methods()
                    .iter()
                    .any(|m| m.get_name().contains("_CompareRepeatedBattleAvatar"))
            {
                return None;
            }
            RuntimeType::from_class(class).ok()
        });

    let Some(handler) = handler else {
        log::debug!("[Method NT] failed to find compare repeated nt");
        return map;
    };

    for method in handler.get_methods_il2cpp() {
        let Ok(m_name) = method.get_name() else {
            continue;
        };
        let m_str = m_name.as_str();
        let Some(inner) = m_str
            .strip_prefix(PREFIX)
            .and_then(|s| s.split('>').next())
            .filter(|s| s.len() != 11 || !s.chars().all(char::is_uppercase))
        else {
            continue;
        };
        let params = method.get_parameters();
        if let Some(param_type) = params
            .first()
            .and_then(|p| super::non_null_type(p.get_parameter_type()))
        {
            let obf_name = param_type.il_name();
            if obf_name.len() == 11
                && obf_name.chars().all(char::is_uppercase)
                && !map.contains_key(obf_name.as_ref())
            {
                log::debug!("[Method NT] param nt: {obf_name} -> {inner}");
                map.insert(obf_name.into_owned(), inner.to_string());
            }
        }
    }

    if map.is_empty() {
        log::debug!("[Method NT] failed to find compare repeated nt");
    }
    map
}

pub fn get_battle_grid_fight_equip_nt() -> HashMap<String, String> {
    let mut map = HashMap::new();

    let Some(c_class) = il2cpp::get_cached_class("RPG.Client.GridFightGameSession.<>c")
        .and_then(|c| RuntimeType::from_class(c).ok())
    else {
        return map;
    };

    for method in c_class.get_methods_il2cpp() {
        if !method.get_name().is_ok_and(|name| {
            name.as_str()
                .contains("_AddStarChangedRoleToBattleChangeEvent")
        }) {
            continue;
        }
        let params = method.get_parameters();
        if let Some(param_type) = params
            .first()
            .and_then(|p| super::non_null_type(p.get_parameter_type()))
        {
            let obf_name = param_type.il_name();
            if obf_name.len() == 11
                && obf_name.chars().all(char::is_uppercase)
                && !map.contains_key(obf_name.as_ref())
            {
                log::debug!("[Method NT] param nt: {obf_name} -> BattleGridFightEquipInfo");
                map.insert(
                    obf_name.into_owned(),
                    "BattleGridFightEquipInfo".to_string(),
                );
            }
        }
    }

    if map.is_empty() {
        log::debug!("[Method NT] failed to find BattleGridFightEquipInfo nt");
    }
    map
}

pub fn get_grid_fight_statistics_nt() -> HashMap<String, String> {
    let mut map = HashMap::new();

    let stats_type = il2cpp::get_cached_class("RPG.GameCore.GridFightManager")
        .and_then(|c| RuntimeType::from_class(c).ok())
        .and_then(|rt| {
            rt.find_method_il2cpp("get_GridFightStatistics")
                .and_then(|method| method.get_return_type().ok())
                .or_else(|| {
                    rt.get_property("GridFightStatistics".into(), 62)
                        .ok()
                        .filter(|property| !property.is_null())
                        .and_then(|property| super::non_null_type(property.get_property_type()))
                })
        });
    let Some(stats_type) = stats_type else {
        return map;
    };

    for method in stats_type.get_methods_il2cpp() {
        let params = method.get_parameters();
        if params.len() == 2
            && method.get_attributes().is_ok_and(|a| {
                a.unbox()
                    .contains(MethodAttributes::Public | MethodAttributes::Static)
            })
            && let Some(p0) = super::non_null_type(params[0].get_parameter_type())
        {
            let obf_name = p0.il_name();
            if obf_name.len() == 11
                && obf_name.chars().all(char::is_uppercase)
                && !map.contains_key(obf_name.as_ref())
            {
                log::debug!("[Method NT] param nt: {obf_name} -> GridFightInfo");
                map.insert(obf_name.into_owned(), "GridFightInfo".to_string());
            }
        }
    }

    if map.is_empty() {
        log::debug!("[Method NT] failed to find GridFightInfo nt");
    }
    map
}
