use crate::proto::{output::TypeToItemMap, recovery::Hints};

pub fn process(items: &TypeToItemMap) -> Hints {
    super::recover_parser(
        items,
        "_ParseServerDispatchData",
        "RPG.Client.ServerDispatchData",
    )
}
