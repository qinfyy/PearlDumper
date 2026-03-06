
use crate::{core::{object::SystemObject, string::SystemString}, cs_class, cs_method};

#[repr(transparent)]
#[derive(Debug, Copy, Clone)]
pub struct GameCoreConfigLoader(pub usize);

impl GameCoreConfigLoader {
    cs_class!(&String::from("RPG.GameCore.GameCoreConfigLoader"));
    cs_method!(pub load_rt_level_floor_info, "LoadRtLevelFloorInfo", &["string"], SystemObject, (value: SystemString));
    cs_method!(pub load_rt_level_group_info, "LoadRtLevelGroupInfo", &["string"], SystemObject, (value: SystemString));
    cs_method!(pub load_level_group_info, "LoadLevelGroupInfo", &["string"], SystemObject, (value: SystemString));
    cs_method!(pub load_level_navmap_config, "LoadLevelNavmapConfig", &["string"], SystemObject, (value: SystemString));
    cs_method!(pub load_summon_unit_config, "LoadSummonUnitConfig", &["string"], SystemObject, (value: SystemString));
    cs_method!(pub load_adventure_character_config, "LoadAdventureCharacterConfig", &["string"], SystemObject, (value: SystemString));
    cs_method!(pub load_level_graph_config, "LoadLevelGraphConfig", &["string"], SystemObject, (value: SystemString));

    pub fn new(ptr: usize) -> Self {
        GameCoreConfigLoader(ptr)
    }
}
