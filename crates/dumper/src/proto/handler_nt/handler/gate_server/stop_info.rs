use crate::proto::{output::TypeToItemMap, recovery::Hints};

pub fn process_stop_info(items: &TypeToItemMap) -> Hints {
    super::recover_parser(items, "_ParseServerStopInfo", "RPG.Client.ServerStopInfo")
}
