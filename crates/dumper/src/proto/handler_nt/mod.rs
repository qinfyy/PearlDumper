use crate::proto::recovery::Hints;
use std::sync::OnceLock;
use std::time::Instant;

use anyhow::Context;

use crate::proto::output::TypeToItemMap;

pub mod handler;

static CACHED_HANDLER_MAP: OnceLock<Hints> = OnceLock::new();
pub static CS_HANDLER_TABLE: OnceLock<Vec<(String, String, usize)>> = OnceLock::new();
// pub static SC_HANDLER_TABLE: OnceLock<Vec<(String, String, usize)>> = OnceLock::new();

pub fn get_handler_nt_map(type_to_item: &TypeToItemMap) -> anyhow::Result<Hints> {
    if let Some(cached) = CACHED_HANDLER_MAP.get() {
        return Ok(cached.clone());
    }

    let mut map = Hints::new();
    let steps: &[(&str, fn(&TypeToItemMap) -> Hints)] = &[
        (
            "avatar path equipment",
            handler::avatar_path_equipment::process,
        ),
        ("equipment", handler::equipment::process),
        ("relic", handler::relic::process),
        ("quest", handler::quest::process),
        (
            "challenge peak notify",
            handler::challengepeak_notify::process,
        ),
        ("scene battle info", handler::scene_battle_info::process),
        ("scene player info", handler::scene_info::process_player),
        ("scene light info", handler::scene_info::process_light),
        ("lineup info", handler::lineup_info::process),
        ("friend login info", handler::friend_login_info::process),
        ("player get token", handler::cs_player_get_token::process),
        ("player sync", handler::player_sync_sc_notify::process),
        ("gate server", handler::gate_server::process_all),
    ];
    for &(name, recover) in steps {
        let started = Instant::now();
        log::info!("[Handler NT] Starting {name}");
        let mut entries = crate::guarded(|| Ok(recover(type_to_item)))
            .with_context(|| format!("packet handler field names: {name}"))?;
        if !matches!(name, "gate server" | "equipment" | "relic" | "quest") {
            entries.advisory("legacy linear register/adjacency heuristic; requires independent scoped confirmation");
        }
        log::info!(
            "[Handler NT] Finished {name}: {} names in {:.2}s",
            entries.len(),
            started.elapsed().as_secs_f64()
        );
        map.extend(entries);
    }
    let _ = CACHED_HANDLER_MAP.set(map.clone());

    Ok(map)
}
