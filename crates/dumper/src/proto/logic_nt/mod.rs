use super::{output::ProtoItem, recovery::Hints};
use std::{cell::RefCell, collections::HashMap, rc::Rc};

pub mod field;
pub mod message;

pub fn run_logic_nt(
    items: &[Rc<RefCell<ProtoItem>>],
    type_names: &HashMap<String, String>,
    field_names: &HashMap<String, String>,
) -> Hints {
    let mut hints = message::deobf_messages(items, type_names);
    // Never feed tentative category names into field recovery.
    hints.extend(field::deobf_fields(items, type_names, field_names));
    hints
}
