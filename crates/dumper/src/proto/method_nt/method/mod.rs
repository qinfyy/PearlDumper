use crate::proto::recovery::Hints;
use std::collections::HashSet;
use std::time::Instant;

use anyhow::Context;
use il2cpp::vm::value::Il2CppValue;
use reflection::runtime_type::RuntimeType;

pub mod avatar;
pub mod battle;
pub mod challenge;
pub mod nt_expected;
pub mod scene;

pub(super) fn get_callback_parameter_nt(mapping: &[(&str, &str)]) -> Hints {
    let mut map = Hints::new();
    // Filter cached native names before allocating any reflection objects. All
    // callback patterns share this pass instead of rescanning every client type.
    for (signature, &method) in il2cpp::FUNCTIONS_TABLE.get().expect("missing method cache") {
        let Some((_, name)) = signature.rsplit_once("::") else {
            continue;
        };
        let deobfuscated: Vec<_> = mapping
            .iter()
            .filter_map(|(pattern, name_out)| name.contains(pattern).then_some(*name_out))
            .collect();
        if deobfuscated.is_empty() {
            continue;
        }
        if method.class().get_image().get_name() != "Assembly-CSharp.dll"
            || il2cpp::api::il2cpp_method_get_param_count(method) == 0
        {
            continue;
        }
        let ty = il2cpp::api::il2cpp_method_get_param(method, 0);
        if ty.is_null() {
            log::warn!("[Method NT] {signature} has a null parameter type");
            continue;
        }
        let obfuscated = ty.il_name();
        if obfuscated.len() == 11 && obfuscated.chars().all(char::is_uppercase) {
            map.context(signature.clone(), method.checked_rva().unwrap_or(0));
            for name in deobfuscated {
                map.insert(obfuscated.to_string(), name.to_owned());
            }
        }
    }
    map
}

pub(super) fn non_null_type(result: anyhow::Result<RuntimeType>) -> Option<RuntimeType> {
    result.ok().filter(|ty| !ty.is_null())
}

pub fn get_method_nt_entries() -> anyhow::Result<Hints> {
    let steps: &[(&str, fn() -> Hints)] = &[
        ("avatar types", avatar::get_avatar_nt),
        ("avatar callbacks", avatar::get_avatar_response_nt),
        ("challenge history", challenge::get_challenge_history_nt),
        ("challenge peak", challenge::get_challengepeak_nt),
        (
            "challenge peak group",
            challenge::get_challenge_peak_group_nt,
        ),
        (
            "challenge tierce handler",
            challenge::get_challenge_tierce_handler_nt,
        ),
        (
            "current tierce challenge",
            challenge::get_cur_tierce_challenge_nt,
        ),
        ("battle send", battle::get_battle_send_nt),
        ("compare repeated", battle::get_compare_repeated_nt),
        (
            "grid fight equipment",
            battle::get_battle_grid_fight_equip_nt,
        ),
        (
            "grid fight statistics",
            battle::get_grid_fight_statistics_nt,
        ),
        ("scene props", scene::get_scene_prop_info_nt),
        ("maze prop state", scene::get_maze_prop_state_nt),
        ("scene monster waves", scene::get_scene_monster_wave_nt),
    ];
    let mut map = Hints::new();
    for &(name, recover) in steps {
        let started = Instant::now();
        log::info!("[Method NT] Starting {name}");
        let mut entries = crate::guarded(|| Ok(recover()))
            .with_context(|| format!("method name recovery: {name}"))?;
        if matches!(
            name,
            "avatar types"
                | "challenge history"
                | "challenge peak group"
                | "current tierce challenge"
                | "grid fight statistics"
                | "scene props"
                | "maze prop state"
        ) {
            entries.advisory("legacy signature/count heuristic; requires independent identity");
        }
        log::info!(
            "[Method NT] Finished {name}: {} names in {:.2}s",
            entries.len(),
            started.elapsed().as_secs_f64()
        );
        map.extend(entries);
    }

    let deobf_set: HashSet<&str> = map.values().map(std::string::String::as_str).collect();
    let missing: Vec<&&str> = nt_expected::EXPECTED_NT
        .iter()
        .filter(|e| !deobf_set.contains(*e))
        .collect();
    if !missing.is_empty() {
        log::debug!("[Method NT] missing: {missing:?}");
    }

    Ok(map)
}
