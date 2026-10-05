use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub mod avatar;
pub mod avatar_property;
pub mod battle_avatar;
pub mod battle_op;
pub mod challenge_boss_relic_info;
pub mod chat_data;
pub mod entered_scene;
pub mod message_type;
pub mod player_heart_beat;
pub mod playerbasic;
pub mod region;
pub mod scene_battle_info;
pub mod scene_buff_info;
pub mod scene_cast_skill;
pub mod scene_entity_info;

use self::message_type::{CATEGORIES, Category};
use super::super::output::{ProtoItem, short_name};
use crate::proto::recovery::Hints;

pub fn deobf_messages(
    items: &[Rc<RefCell<ProtoItem>>],
    method_nt_map: &HashMap<String, String>,
) -> Hints {
    let mut nt_map = Hints::new();
    let mut current_cat: Option<&Category> = None;
    let mut pending_enums = Vec::new();
    let mut pending_items = Vec::new();

    let flush_category = |cat: &Category,
                          enums: &mut Vec<String>,
                          p_items: &mut Vec<Rc<RefCell<ProtoItem>>>,
                          nt_map: &mut Hints| {
        nt_map.context(
            format!("legacy category {}..{}", cat.trigger, cat.target),
            0,
        );
        if enums.len() == cat.enums.len() {
            for (idx, obf_name) in enums.iter().enumerate() {
                if cat.enums[idx] != "Obf" {
                    nt_map.insert(obf_name.clone(), cat.enums[idx].to_string());
                }
            }
        } else {
            log::debug!(
                "[logic_nt] category trigger={} enums mismatch: expected {} got {} ({} → {})",
                cat.trigger,
                cat.enums.len(),
                enums.len(),
                cat.enums.join(", "),
                enums.join(", "),
            );
        }
        if p_items.len() == cat.messages.len() {
            for (idx, item) in p_items.iter().enumerate() {
                let item_ref = item.borrow();
                let obf_name = match &*item_ref {
                    ProtoItem::Message(m) => &m.name,
                    ProtoItem::Enum(e) => &e.name,
                };
                if cat.messages[idx] != "Obf" {
                    nt_map.insert(obf_name.clone(), cat.messages[idx].to_string());
                }
            }
        } else {
            log::debug!(
                "[logic_nt] category trigger={} messages mismatch: expected {} got {} ({} → {})",
                cat.trigger,
                cat.messages.len(),
                p_items.len(),
                cat.messages.join(", "),
                p_items
                    .iter()
                    .map(|i| match &*i.borrow() {
                        ProtoItem::Message(m) => m.name.clone(),
                        ProtoItem::Enum(e) => e.name.clone(),
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
            );
        }
        enums.clear();
        p_items.clear();
    };

    for item in items {
        let item_ref = item.borrow();
        let (name, deobf_name_field) = match &*item_ref {
            ProtoItem::Message(m) => (&m.name, &m.deobfuscated_name),
            ProtoItem::Enum(e) => (&e.name, &e.deobfuscated_name),
        };

        let deobf_name = deobf_name_field
            .clone()
            .or_else(|| method_nt_map.get(name).cloned());

        let short_deobf = deobf_name.as_deref().map(short_name);

        // An anchor can close one category and open the next, such as AvatarProperty.
        if let Some(cat) = current_cat.filter(|c| short_deobf == Some(c.target)) {
            flush_category(cat, &mut pending_enums, &mut pending_items, &mut nt_map);
            current_cat = None;
        }

        if let Some(next_cat) =
            short_deobf.and_then(|sd| CATEGORIES.iter().find(|c| c.trigger == sd))
        {
            if let Some(prev_cat) = current_cat {
                // A missing end anchor must not turn unrelated types into this category.
                log::debug!(
                    "[logic_nt] skipped unterminated category {} before {}",
                    prev_cat.trigger,
                    next_cat.trigger
                );
                pending_enums.clear();
                pending_items.clear();
            }
            current_cat = Some(next_cat);
            continue;
        }

        if current_cat.is_some() {
            match &*item_ref {
                ProtoItem::Enum(e) if !e.name.contains('.') => pending_enums.push(e.name.clone()),
                ProtoItem::Message(m) if !m.has_parent => pending_items.push(item.clone()),
                _ => {}
            }
        }
    }

    if let Some(cat) = current_cat {
        log::debug!(
            "[logic_nt] skipped unterminated category {} at end of metadata",
            cat.trigger
        );
    }
    recover_chat_messages(items, method_nt_map, &mut nt_map);
    nt_map.advisory("legacy category/number rule; requires independent identity");
    nt_map
}

fn recover_chat_messages(
    items: &[Rc<RefCell<ProtoItem>>],
    known: &HashMap<String, String>,
    nt_map: &mut Hints,
) {
    let resolve = |kind: &str| {
        known
            .get(kind)
            .map(String::as_str)
            .unwrap_or(kind)
            .to_owned()
    };
    let chat_data: Vec<_> = items
        .iter()
        .filter_map(|item| {
            let item = item.borrow();
            let ProtoItem::Message(message) = &*item else {
                return None;
            };
            (!message.has_parent
                && message.fields.len() == 2
                && message.fields.iter().any(|field| field.number == 1)
                && message
                    .fields
                    .iter()
                    .any(|field| field.number == 2 && resolve(&field.kind) == "ChatData"))
            .then(|| message.name.clone())
        })
        .collect();
    let [chat_data] = chat_data.as_slice() else {
        return;
    };
    let chat_messages: Vec<_> = items
        .iter()
        .filter_map(|item| {
            let item = item.borrow();
            let ProtoItem::Message(message) = &*item else {
                return None;
            };
            let field = |number| message.fields.iter().find(|field| field.number == number);
            let sender = field(10)?;
            let receiver = field(11)?;
            (!message.has_parent
                && field(9)?.kind == format!("repeated {chat_data}")
                && sender.kind == receiver.kind)
                .then(|| (message.name.clone(), sender.kind.clone()))
        })
        .collect();
    let [(chat_message, contact)] = chat_messages.as_slice() else {
        return;
    };
    let contacts = items
        .iter()
        .filter(|item| {
            let item = item.borrow();
            let ProtoItem::Message(message) = &*item else {
                return false;
            };
            (message.name == *contact || message.deobfuscated_name.as_deref() == Some(contact))
                && message.fields.len() == 2
                && message
                    .fields
                    .iter()
                    .any(|field| field.number == 2 && field.kind == "uint32")
        })
        .count();
    if contacts != 1 {
        return;
    }
    for (obfuscated, name) in [
        (chat_data, "MessageChatData"),
        (contact, "ContactIdentity"),
        (chat_message, "ChatMessageData"),
    ] {
        nt_map.context("legacy chat field-number chain", 0);
        nt_map.insert(obfuscated.clone(), name.to_owned());
    }
}
