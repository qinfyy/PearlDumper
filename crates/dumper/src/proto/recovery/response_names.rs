use std::collections::{BTreeMap, BTreeSet};

use super::{Evidence, Hints, readable_type, valid_name};

pub(crate) fn diagnostic_name(literal: &str, callback: &str) -> Option<String> {
    let name = literal.strip_prefix("Proto null, ")?;
    // Some client callbacks spell a response CsRsp. Only the verified diagnostic,
    // never that callback spelling alone, can supply the complete ScRsp name.
    let matches_callback = name.ends_with(callback)
        || callback
            .strip_suffix("CsRsp")
            .is_some_and(|stem| name.ends_with(&format!("{stem}ScRsp")));
    // Only a full protocol token in the failed-argument diagnostic is accepted.
    (valid_name(name)
        && matches_callback
        && (name.ends_with("ScRsp") || name.ends_with("ScNotify")))
    .then(|| name.to_owned())
}

pub(crate) fn normalize(hints: &mut Hints) {
    for c in &mut hints.entries {
        if c.evidence == Evidence::Native && c.name == "PvebattleResultScRsp" {
            c.original_name.get_or_insert_with(|| c.name.clone());
            c.name = "PVEBattleResultScRsp".into();
            c.source
                .push_str("; response spelling alias Pvebattle -> PVEBattle");
        }
    }
    let mut by_name = BTreeMap::<String, BTreeSet<String>>::new();
    for c in &hints.entries {
        if c.evidence == Evidence::Native && readable_type(&c.name) {
            by_name
                .entry(c.name.clone())
                .or_default()
                .insert(c.symbol.clone());
        }
    }
    for c in &mut hints.entries {
        if c.evidence != Evidence::Native
            || !by_name
                .get(&c.name)
                .is_some_and(|symbols| symbols.len() > 1)
        {
            continue;
        }
        let Some((owner, _)) = c.method.as_deref().and_then(|m| m.split_once("::")) else {
            continue;
        };
        let Some(owner) = owner.strip_prefix("RPG.Client.") else {
            continue;
        };
        let scope = owner.replace('.', "_");
        if !valid_name(&scope) {
            continue;
        }
        c.original_name.get_or_insert_with(|| c.name.clone());
        c.source.push_str("; declaring-type-qualified output alias for a reused callback; not an official protocol name");
        c.name = format!("{scope}_{}", c.name);
    }
}
