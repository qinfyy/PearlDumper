use crate::proto::{output::TypeToItemMap, recovery::Hints};

pub fn process(items: &TypeToItemMap) -> Hints {
    super::sync::process(items, "RPG.Client.EquipmentItemData", "Sync")
}
