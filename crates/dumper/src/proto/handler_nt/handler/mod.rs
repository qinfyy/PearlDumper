use std::collections::{BTreeSet, HashMap};

use il2cpp::vm::value::Il2CppValue;
use reflection::runtime_type::RuntimeType;

use crate::proto::util::is_correct_getter;

pub mod avatar_path_equipment;
pub mod challengepeak_notify;
pub mod cs_player_get_token;
pub mod equipment;
pub mod friend_login_info;
pub mod gate_server;
pub mod lineup_info;
pub mod player_sync_sc_notify;
pub mod quest;
pub mod relic;
pub mod scene_battle_info;
pub mod scene_info;

pub fn build_game_offset_map(rt: RuntimeType) -> HashMap<usize, String> {
    build_game_offset_candidates(rt)
        .into_iter()
        .filter_map(|(offset, names)| {
            (names.len() == 1).then(|| (offset, names.into_iter().next().unwrap()))
        })
        .collect()
}

pub fn build_game_offset_candidates(rt: RuntimeType) -> HashMap<usize, BTreeSet<String>> {
    use crate::proto::{output::snake_field, recovery::readable};
    let fields = rt.get_fields_il2cpp();
    let mut map = HashMap::<usize, BTreeSet<String>>::new();
    for prop in rt.get_properties(62) {
        if let (Ok(getter), Ok(name)) = (prop.get_get_method(true), prop.get_name()) {
            if getter.is_null() {
                continue;
            }
            let Ok(rva) = getter.get_il2cpp_method().checked_rva() else {
                continue;
            };
            if let Some(f) = fields
                .iter()
                .find(|f| is_correct_getter(f.get_offset(), rva))
            {
                if readable(&name.as_str()) {
                    map.entry(f.get_offset())
                        .or_default()
                        .insert(snake_field(&name.as_str()));
                }
            }
        }
    }

    for f in fields {
        if let Ok(name) = f.get_name() {
            let n = name.as_str();
            let clean = n
                .strip_prefix('<')
                .and_then(|s| s.strip_suffix(">k__BackingField"))
                .unwrap_or(&n);
            if readable(clean) {
                map.entry(f.get_offset())
                    .or_default()
                    .insert(snake_field(clean));
            }
        }
    }

    map
}
