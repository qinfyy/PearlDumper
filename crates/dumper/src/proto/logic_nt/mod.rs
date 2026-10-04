use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub mod field;
pub mod message;

use super::output::ProtoItem;

pub struct NameRecovery {
    pub types: HashMap<String, String>,
    pub fields: HashMap<String, String>,
}

pub fn run_logic_nt(
    items: &[Rc<RefCell<ProtoItem>>],
    message_nt_map: &HashMap<String, String>,
    predeobf_map: &HashMap<String, String>,
) -> anyhow::Result<NameRecovery> {
    let mut nt_map: indexmap::IndexMap<String, String> = indexmap::IndexMap::new();
    let mut global_field_map: HashMap<String, String> = predeobf_map.clone();

    for (k, v) in predeobf_map {
        nt_map.entry(k.clone()).or_insert(v.clone());
    }

    for (obf_name, deobf_name) in message_nt_map {
        nt_map.insert(obf_name.clone(), deobf_name.clone());
    }

    message::deobf_messages(items, message_nt_map, &mut nt_map);
    field::deobf_fields(items, &mut nt_map, &mut global_field_map);

    if !nt_map.is_empty() {
        let output_content: String = nt_map
            .iter()
            .map(|(k, v)| format!("{k} {v}"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write("./DUMP/nt.txt.partial", output_content)?;
        std::fs::rename("./DUMP/nt.txt.partial", "./DUMP/nt.txt")?;
    }
    // nt.txt mixes fields and types. Only actual type identities may rename definitions.
    let types = items
        .iter()
        .filter_map(|item| {
            let item = item.borrow();
            let name = match &*item {
                ProtoItem::Message(message) => &message.name,
                ProtoItem::Enum(en) => &en.name,
            };
            nt_map
                .get(name)
                .map(|recovered| (name.clone(), recovered.clone()))
        })
        .collect();
    Ok(NameRecovery {
        types,
        fields: global_field_map,
    })
}
