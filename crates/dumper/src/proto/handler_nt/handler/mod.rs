use std::collections::{BTreeSet, HashMap};

use il2cpp::vm::value::Il2CppValue;
use reflection::runtime_type::RuntimeType;

use crate::proto::recovery::{
    Candidate, Evidence, Hints,
    accessor::{direct_field, storage_name},
};

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
mod sync;

pub struct GameFieldName {
    name: String,
    original: String,
    source: String,
    width: Option<usize>,
}

fn storage_width(ty: RuntimeType) -> Option<usize> {
    Some(match ty.il_name().as_ref() {
        "System.Boolean" | "System.Byte" | "System.SByte" => 1,
        "System.Char" | "System.Int16" | "System.UInt16" => 2,
        "System.Int32" | "System.UInt32" | "System.Single" => 4,
        "System.Int64" | "System.UInt64" | "System.Double" | "System.IntPtr" | "System.UIntPtr" => {
            8
        }
        _ if ty.get_isenum().ok()?.unbox() => {
            return storage_width(reflection::r#enum::Enum::get_underlying_type(ty).ok()?);
        }
        _ if !ty.get_isvaluetype().ok()?.unbox() => 8,
        _ => return None,
    })
}

impl GameFieldName {
    fn submit(
        &self,
        hints: &mut Hints,
        key: &str,
        method: &str,
        rva: usize,
        copy: &crate::proto::recovery::flow::FieldCopy,
    ) {
        if self.width != Some(copy.width) {
            return;
        }
        let instruction = copy.instruction - *il2cpp::GA_BASE;
        hints.entries.push(Candidate {
            symbol: key.into(),
            name: self.name.clone(),
            original_name: Some(self.original.clone()),
            source: format!("native field copy; {}", self.source),
            method: Some(format!("{method}; copy at RVA 0x{instruction:X}")),
            rva: Some(rva),
            evidence: Evidence::Native,
        });
    }
}

pub fn build_game_offset_map(rt: RuntimeType) -> HashMap<usize, String> {
    build_game_offset_candidates(rt)
        .into_iter()
        .filter_map(|(offset, names)| {
            let names: BTreeSet<_> = names.into_iter().map(|n| n.name).collect();
            (names.len() == 1).then(|| (offset, names.into_iter().next().unwrap()))
        })
        .collect()
}

pub fn build_game_offset_candidates(rt: RuntimeType) -> HashMap<usize, Vec<GameFieldName>> {
    use crate::proto::{output::snake_field, recovery::readable};
    let fields: Vec<_> = rt
        .get_fields_il2cpp()
        .into_iter()
        .filter(|f| f.is_instance())
        .collect();
    let mut map = HashMap::<usize, Vec<GameFieldName>>::new();
    for prop in rt.get_properties(62) {
        if let (Ok(getter), Ok(name)) = (prop.get_get_method(true), prop.get_name()) {
            if getter.is_null() || getter.is_static() || !getter.get_parameters().is_empty() {
                continue;
            }
            let Ok(rva) = getter.get_il2cpp_method().checked_rva() else {
                continue;
            };
            let Ok(table) = utils::scanner::game_assembly_functions() else {
                continue;
            };
            if table
                .containing(rva)
                .is_some_and(|range| range.start as usize != rva)
            {
                continue;
            }
            // Leaf getters lack unwind records. Read only the executable section,
            // bounded to 64 bytes; direct_field must observe a terminating RET.
            let Some(bytes) =
                utils::scanner::game_assembly_code_sections().find_map(|(start, bytes)| {
                    let offset = rva.checked_sub(start)?;
                    bytes.get(offset..offset.saturating_add(64).min(bytes.len()))
                })
            else {
                continue;
            };
            let end = table.containing(rva).map_or(bytes.len(), |range| {
                (range.end as usize - rva).min(bytes.len())
            });
            let Some((offset, width)) = direct_field(&bytes[..end]) else {
                continue;
            };
            let Ok(property_type) = prop.get_property_type() else {
                continue;
            };
            if storage_width(property_type) != Some(width) {
                continue;
            }
            if let Some(f) = fields.iter().find(|f| {
                f.get_offset() == offset && f.get_field_type().is_ok_and(|ty| ty == property_type)
            }) {
                if readable(&name.as_str()) {
                    map.entry(f.get_offset()).or_default().push(GameFieldName {
                        name: snake_field(&name.as_str()),
                        original: name.as_str().into_owned(),
                        width: Some(width),
                        source: format!(
                            "{}::get_{} RVA 0x{rva:X}; direct accessor of offset 0x{offset:X}",
                            rt.il_name(),
                            name.as_str()
                        ),
                    });
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
                let names = map.entry(f.get_offset()).or_default();
                let canonical = storage_name(
                    &n,
                    &names.iter().map(|n| n.name.clone()).collect::<Vec<_>>(),
                );
                names.push(GameFieldName {
                    name: canonical,
                    original: n.into_owned(),
                    width: f.get_field_type().ok().and_then(storage_width),
                    source: format!(
                        "{} field metadata at offset 0x{:X}",
                        rt.il_name(),
                        f.get_offset()
                    ),
                });
            }
        }
    }

    map
}
