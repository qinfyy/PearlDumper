use reflection::attributes::MethodAttributes;
use std::collections::HashMap;

use il2cpp::vm::value::Il2CppValue;
use reflection::runtime_type::RuntimeType;

pub fn get_challenge_history_nt() -> HashMap<String, String> {
    let mut map = HashMap::new();

    let history_type = il2cpp::get_cached_class("RPG.Client.ChallengeModule")
        .and_then(|c| RuntimeType::from_class(c).ok())
        .and_then(|rt| {
            rt.get_methods_il2cpp()
                .into_iter()
                .find(|m| {
                    m.get_name()
                        .is_ok_and(|n| n.as_str() == "get_ChallengeHistory")
                })
                .and_then(|m| m.get_return_type().ok())
        });
    let Some(history_type) = history_type else {
        return map;
    };

    for method in history_type.get_methods_il2cpp() {
        let params = method.get_parameters();
        if params.len() == 1
            && let Some(param_type) = super::non_null_type(params[0].get_parameter_type())
            && param_type.il_name().contains("IEnumerable")
            && let Some(inner) = param_type.get_generic_arguments().into_iter().next()
        {
            let obf_name = inner.il_name().into_owned();
            if obf_name.len() == 11 && obf_name.chars().all(char::is_uppercase) {
                log::debug!("[Method NT] param nt: {obf_name} -> ChallengeHistoryMaxLevel");
                map.insert(obf_name, "ChallengeHistoryMaxLevel".to_string());
            }
        }
    }

    if map.is_empty() {
        log::debug!("[Method NT] failed to find ChallengeHistoryMaxLevel nt");
    }
    map
}

pub fn get_challengepeak_nt() -> HashMap<String, String> {
    super::get_callback_parameter_nt(&[("<_GetPeakData", "ChallengePeak")])
}

pub fn get_challenge_peak_group_nt() -> HashMap<String, String> {
    let mut map = HashMap::new();

    let server_agent_type = il2cpp::get_cached_class("RPG.Client.ChallengePeakModule")
        .and_then(|c| RuntimeType::from_class(c).ok())
        .and_then(|rt| rt.get_field("_ServerAgent".into(), 62).ok())
        .filter(|field| !field.is_null())
        .and_then(|f| super::non_null_type(f.get_field_type()));
    let Some(server_agent_type) = server_agent_type else {
        return map;
    };

    for field in server_agent_type.get_fields_il2cpp() {
        if let Some(ft) = super::non_null_type(field.get_field_type()) {
            let ft_name = ft.il_name();
            if !ft_name.contains("Dictionary") {
                continue;
            }
            let generics = ft.get_generic_arguments();
            if let Some(second) = generics.into_iter().nth(1) {
                let obf_name = second.il_name().into_owned();
                if obf_name.len() == 11 && obf_name.chars().all(char::is_uppercase) {
                    log::debug!("[Method NT] param nt: {obf_name} -> ChallengePeakGroup");
                    map.insert(obf_name, "ChallengePeakGroup".to_string());
                }
            }
        }
    }

    if map.is_empty() {
        log::debug!("[Method NT] failed to find ChallengePeakGroup handler nt");
    }
    map
}

pub fn get_cur_tierce_challenge_nt() -> HashMap<String, String> {
    let mut map = HashMap::new();

    let repo_type = il2cpp::get_cached_class(
        "RPG.Client.Challenge.Tierce.ChallengeTierceStageSelectViewModel.Factory",
    )
    .and_then(|c| RuntimeType::from_class(c).ok())
    .and_then(|rt| rt.get_field("_Repository".into(), 62).ok())
    .filter(|field| !field.is_null())
    .and_then(|f| super::non_null_type(f.get_field_type()));
    let Some(repo_type) = repo_type else {
        return map;
    };

    for method in repo_type.get_methods_il2cpp() {
        let params = method.get_parameters();
        if params.len() == 2
            && !params
                .iter()
                .any(|p| p.get_is_out().is_ok_and(|b| b.unbox()))
            && method
                .get_attributes()
                .is_ok_and(|a| !a.unbox().contains(MethodAttributes::Public))
            && let (Some(t0), Some(t1)) = (
                super::non_null_type(params[0].get_parameter_type()),
                super::non_null_type(params[1].get_parameter_type()),
            )
        {
            let n0 = t0.il_name();
            let n1 = t1.il_name();
            if n0.len() == 11
                && n0.chars().all(char::is_uppercase)
                && n1.len() == 11
                && n1.chars().all(char::is_uppercase)
            {
                log::debug!("[Method NT] param nt: {n0} -> CurTierceChallenge");
                map.insert(n0.into_owned(), "CurTierceChallenge".to_string());
                break;
            }
        }
    }

    if map.is_empty() {
        log::debug!("[Method NT] failed to find CurTierceChallenge nt");
    }
    map
}

pub fn get_challenge_tierce_handler_nt() -> HashMap<String, String> {
    let mut map = HashMap::new();

    let classes = il2cpp::CLASS_TABLE_VEC.get().expect("missing class cache");
    let handler_class = classes[unsafe { il2cpp::ASSEMBLY_CSHARP_START } as usize..]
        .iter()
        .find_map(|&class| {
            let fields = class.get_fields();
            let has_big = fields.iter().any(|f| f.get_name() == "OnBigSettle");
            let has_medal = fields.iter().any(|f| f.get_name() == "OnMedal");
            let has_clear = fields.iter().any(|f| f.get_name() == "OnStageClear");
            if has_big && has_medal && has_clear {
                RuntimeType::from_class(class).ok()
            } else {
                None
            }
        });

    let Some(handler) = handler_class else {
        log::debug!("[Method NT] failed to find challenge tierce handler nt");
        return map;
    };

    log::debug!(
        "[Method NT] handler: {} -> ChallengeTierceHandler",
        handler.il_name()
    );

    for method in handler.get_methods_il2cpp() {
        let params = method.get_parameters();
        if let (Ok(m_name), Some(param_type)) = (
            method.get_name(),
            params
                .first()
                .and_then(|p| super::non_null_type(p.get_parameter_type())),
        ) {
            let m_name_str = m_name.as_str();
            let obf_name =
                if let Some(generic) = param_type.get_generic_arguments().into_iter().next() {
                    generic.il_name()
                } else {
                    param_type.il_name()
                };

            if obf_name.len() == 11
                && obf_name.chars().all(char::is_uppercase)
                && !map.contains_key(obf_name.as_ref())
                && let Some(deobf_name) = m_name_str
                    .strip_prefix('<')
                    .and_then(|m| m.split('>').next())
                    .filter(|m| m.len() != 11 || !m.chars().all(char::is_uppercase))
                    .map(|m| format!("{m}ChallengeTierceScRsp"))
                    .or_else(|| {
                        m_name_str
                            .strip_prefix("add_On")
                            .map(|e| format!("ChallengeTierce{e}"))
                    })
            {
                log::debug!("[Method NT] param nt: {obf_name} -> {deobf_name}");
                map.insert(obf_name.into_owned(), deobf_name);
            }
        }
    }

    if map.len() <= 1 {
        log::debug!("[Method NT] failed to find challenge tierce nts");
    }
    map
}
