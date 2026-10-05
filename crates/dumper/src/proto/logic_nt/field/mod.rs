use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub mod avatar;
pub mod avatar_battle_info;
pub mod avatar_property;
pub mod battle_avatar;
pub mod battle_avatar_global_buff_info;
pub mod battle_grid_fight_special_battle_info;
pub mod battle_op;
pub mod challenge_boss_relic_info;
pub mod chat_data;
pub mod deobf_map_first;
pub mod deobf_map_second;
pub mod deobf_map_third;
pub mod init_deobf;
pub mod playerbasic;
pub mod region_info;
pub mod scene_info;

use super::super::output::ProtoItem;

pub enum FieldLogic {
    ByNumber(&'static [(u32, &'static str)]),
    ByWireType(&'static [(&'static str, &'static str)]),
}

pub fn deobf_fields(
    items: &[Rc<RefCell<ProtoItem>>],
    type_names: &HashMap<String, String>,
    known_fields: &HashMap<String, String>,
) -> crate::proto::recovery::Hints {
    use crate::proto::recovery::{Evidence, Hints, candidate, field_key};
    let field_configs = [
        challenge_boss_relic_info::CHALLENGE_BOSS_RELIC_INFO_FIELD_MAP,
        chat_data::CHATDATA_FIELD_MAP,
        playerbasic::PLAYER_FIELD_MAP,
        region_info::REGION_INFO_FIELD_MAP,
        battle_op::BATTLE_OP_FIELD_MAP,
        battle_avatar::BATTLE_AVATAR_FIELD_MAP,
        avatar_property::AVATAR_PROPERTY_FIELD_MAP,
        avatar_battle_info::AVATAR_BATTLE_INFO_FIELD_MAP,
        battle_grid_fight_special_battle_info::BATTLE_GRID_FIGHT_SPECIAL_BATTLE_INFO_FIELD_MAP,
        battle_avatar_global_buff_info::BATTLE_AVATAR_GLOBAL_BUFF_INFO_FIELD_MAP,
        init_deobf::INIT_DEOBF_FIELD_MAP,
        deobf_map_first::DEOBF_MAP1_FIELD_MAP,
        avatar::AVATAR_FIELD_MAP,
        deobf_map_second::DEOBF_MAP_SECOND_FIELD_MAP,
        deobf_map_third::DEOBF_MAP3_FIELD_MAP,
        scene_info::SCENE_INFO_FIELD_MAP,
    ];

    let mut hints = Hints::new();
    for item in items {
        let borrowed = item.borrow();
        let ProtoItem::Message(m) = &*borrowed else {
            continue;
        };
        let Some(name) = type_names.get(&m.name) else {
            continue;
        };
        let fields: Vec<_> = m
            .fields
            .iter()
            .chain(m.oneofs.iter().flat_map(|o| &o.fields))
            .collect();
        for (_, logic) in field_configs
            .iter()
            .flat_map(|c| c.iter())
            .filter(|(n, _)| *n == name)
        {
            match logic {
                FieldLogic::ByNumber(mappings) => {
                    // Numbers alone are not stable semantic identities across versions.
                    let valid = mappings.iter().all(|(n, expected)| {
                        fields.iter().any(|f| f.number == *n)
                            && known_fields
                                .get(&field_key(&m.name, *n))
                                .is_some_and(|v| v == expected)
                    });
                    for (n, expected) in *mappings {
                        if fields.iter().any(|f| f.number == *n) {
                            hints.entries.push(candidate(
                                &field_key(&m.name, *n),
                                expected,
                                "number rule: requires scoped structure/name confirmation",
                                if valid {
                                    Evidence::Structural
                                } else {
                                    Evidence::Hint
                                },
                            ));
                        }
                    }
                }
                FieldLogic::ByWireType(mappings) => {
                    for (kind, expected) in *mappings {
                        let matches: Vec<_> = fields
                            .iter()
                            .filter(|f| {
                                crate::proto::output::rewrite_type_tokens(&f.kind, |s| {
                                    type_names.get(s).cloned().unwrap_or_else(|| s.into())
                                }) == *kind
                            })
                            .collect();
                        let unique = matches.len() == 1
                            && mappings.iter().filter(|(k, _)| k == kind).count() == 1;
                        for f in matches {
                            hints.entries.push(candidate(
                                &field_key(&m.name, f.number),
                                expected,
                                "scoped semantic wire-type rule",
                                if unique {
                                    Evidence::Structural
                                } else {
                                    Evidence::Hint
                                },
                            ));
                        }
                    }
                }
            }
        }
    }
    hints
}
