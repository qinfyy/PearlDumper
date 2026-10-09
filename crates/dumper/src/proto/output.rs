use super::{MessageMinimalInfo, cache::TypeCache};

use crate::proto::{NumberType, RETCODE_FIELD_NAME, cache::CachedType};
use anyhow::{Context, ensure};
use convert_case::{Case, Casing as _};
use il2cpp::vm::{metadata_cache, object::Il2CppObject, string::Il2CppString, value::Il2CppValue};
use indexmap::IndexMap;
use reflection::runtime_type::RuntimeType;
use std::{
    borrow::Cow,
    cell::RefCell,
    collections::{HashMap, HashSet},
    io::Write,
    rc::Rc,
};

pub type TypeToItemMap = IndexMap<RuntimeType, Rc<RefCell<ProtoItem>>>;

#[derive(Debug, PartialEq, Eq)]
pub enum MessageType {
    None,
    Req,
    Rsp,
    Notify,
}

#[derive(Debug)]
pub struct Message {
    pub cmd_id: u16,
    pub name: String,
    pub deobfuscated_name: Option<String>,
    pub fields: Vec<Field>,
    pub oneofs: Vec<OneOf>,
    pub children: Vec<Rc<RefCell<ProtoItem>>>,
    pub has_parent: bool,
    pub msg_type: MessageType,
    pub write_to_rva: usize,
    pub merge_from_rva: usize,
}
#[derive(Debug)]

pub struct Enum {
    pub name: String,
    pub deobfuscated_name: Option<String>,
    pub variants: Vec<(String, i32)>,
    pub has_parent: bool,
}
#[derive(Debug)]
pub struct Field {
    pub kind: String,
    pub name: String,
    pub number: u32,
    pub offset: u32,
}

#[derive(Debug)]

pub struct OneOf {
    pub name: String,
    pub fields: Vec<Field>,
}

#[derive(Debug)]
pub enum ProtoItem {
    Message(Message),
    Enum(Enum),
}

impl Message {
    pub fn fmt_protobuf_with_depth(&self, nested_depth: usize) -> String {
        let mut result = String::new();

        let indent = "\t".repeat(nested_depth);

        if self.deobfuscated_name.is_some() {
            result.push_str(&format!("// Obf: {}\n", self.name));
        }

        if self.msg_type != MessageType::None {
            result.push_str(&format!("// Type: {:#?}\n", self.msg_type));
        }

        if self.cmd_id != 0 {
            result.push_str(&format!("// CmdID: {}\n", self.cmd_id));
        }

        if self.merge_from_rva != 0 || self.write_to_rva != 0 {
            result.push_str(&format!(
                "// WriteTo: 0x{:X} | MergeFrom: 0x{:X}\n",
                self.write_to_rva, self.merge_from_rva
            ));
        }

        result.push_str(&format!(
            "{}message {} {{",
            indent,
            if let Some(deobf_name) = &self.deobfuscated_name {
                short_name(deobf_name)
            } else {
                short_name(&self.name)
            }
        ));
        result.push('\n');

        for nested in &self.children {
            result.push_str(&nested.borrow().fmt_protobuf_with_depth(nested_depth + 1));
        }

        for field in &self.fields {
            let field_display_name = if field.name == RETCODE_FIELD_NAME {
                "retcode".to_string()
            } else {
                snake_field(&field.name)
            };

            result.push_str(&format!(
                "{}\t{} {} = {}; // offset: {}\n",
                indent,
                qualified_field_type(&field.kind),
                field_display_name,
                field.number,
                field.offset
            ));
        }

        for oneof in &self.oneofs {
            result.push_str(&format!(
                "{}\toneof {} {{\n",
                indent,
                snake_field(oneof.name.strip_suffix("Case").unwrap_or(&oneof.name))
            ));

            for field in &oneof.fields {
                let field_display_name = if field.name == RETCODE_FIELD_NAME {
                    "retcode".to_string()
                } else {
                    snake_field(&field.name)
                };

                if field.offset != 0 {
                    result.push_str(&format!(
                        "{}\t\t{} {} = {}; // offset: {}\n",
                        indent,
                        qualified_field_type(&field.kind),
                        field_display_name,
                        field.number,
                        field.offset
                    ));
                } else {
                    result.push_str(&format!(
                        "{}\t\t{} {} = {};\n",
                        indent,
                        qualified_field_type(&field.kind),
                        field_display_name,
                        field.number
                    ));
                }
            }
            result.push_str(&format!("{indent}\t}}\n"));
        }

        result.push_str(&format!("{indent}}}\n"));

        result
    }
}

impl Enum {
    pub fn fmt_protobuf_with_depth(&self, nested_depth: usize) -> String {
        let mut result = String::new();

        let indent = "\t".repeat(nested_depth);

        if self.deobfuscated_name.is_some() {
            result.push_str(&format!("// Obf: {}\n", self.name));
        }

        result.push_str(&format!(
            "{}enum {} {{",
            indent,
            if let Some(deobf_name) = self.deobfuscated_name.as_ref() {
                short_name(deobf_name)
            } else {
                short_name(&self.name)
            }
        ));
        result.push('\n');

        if self
            .variants
            .iter()
            .map(|(_, value)| value)
            .collect::<HashSet<_>>()
            .len()
            != self.variants.len()
        {
            result.push_str(&format!("{indent}\toption allow_alias = true;\n"));
        }
        for (name, discriminant) in &self.variants {
            result.push_str(&format!("{indent}\t{name} = {discriminant};"));
            result.push('\n');
        }

        result.push_str(&format!("{indent}}}\n"));
        result
    }
}

impl ProtoItem {
    pub fn fmt_protobuf_with_depth(&self, nested_depth: usize) -> String {
        match self {
            ProtoItem::Message(message) => message.fmt_protobuf_with_depth(nested_depth),
            ProtoItem::Enum(enumeration) => enumeration.fmt_protobuf_with_depth(nested_depth),
        }
    }
}

pub fn short_name(full_name: &str) -> &str {
    full_name.rsplit('.').next().unwrap_or(full_name)
}

pub fn snake_field(input: &str) -> String {
    let is_all_upper = input
        .chars()
        .all(|c| c.is_uppercase() || !c.is_alphabetic());
    if input.len() == 11 && is_all_upper {
        input.to_string()
    } else {
        input.to_case(Case::Snake)
    }
}

fn qualified_field_type(kind: &str) -> String {
    // Keep internal names compatible with the naming handlers. Only serialized
    // references become absolute, so a nested type cannot shadow a root message.
    rewrite_type_tokens(kind, |name| {
        if matches!(
            name,
            "repeated"
                | "map"
                | "bool"
                | "int32"
                | "int64"
                | "uint32"
                | "uint64"
                | "sint32"
                | "sint64"
                | "fixed32"
                | "fixed64"
                | "sfixed32"
                | "sfixed64"
                | "float"
                | "double"
                | "string"
                | "bytes"
        ) {
            name.to_owned()
        } else {
            format!(".{}", name.trim_start_matches('.'))
        }
    })
}

pub(super) fn rewrite_type_tokens(kind: &str, mut rename: impl FnMut(&str) -> String) -> String {
    kind.split_inclusive(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '.'))
        .map(|part| {
            let end = part
                .trim_end_matches(|ch: char| {
                    !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '.')
                })
                .len();
            if end == 0 {
                part.to_owned()
            } else {
                format!("{}{}", rename(&part[..end]), &part[end..])
            }
        })
        .collect()
}

fn process_cmd_id(
    map: &mut TypeToItemMap,
    rsp_notify_map: &HashMap<String, u16>,
    req_map: &HashMap<String, (u16, Option<String>)>,
    rsp_notify_names: &HashMap<String, String>,
    method_nt_map: &HashMap<String, String>,
) -> (HashMap<i32, String>, HashMap<String, String>) {
    let mut proposed: HashMap<String, HashSet<String>> = HashMap::new();
    for (obfuscated, name) in rsp_notify_names.iter().chain(method_nt_map) {
        proposed
            .entry(obfuscated.clone())
            .or_default()
            .insert(name.clone());
    }
    for (obfuscated, (_, name)) in req_map {
        if let Some(name) = name {
            proposed
                .entry(obfuscated.clone())
                .or_default()
                .insert(name.clone());
        }
    }
    let mut names = HashMap::new();
    for (obfuscated, candidates) in proposed {
        if candidates.len() == 1 {
            names.insert(obfuscated, candidates.into_iter().next().unwrap());
        } else {
            log::debug!("NT checkpoint has conflicting names for {obfuscated}: {candidates:?}");
        }
    }

    let originals: Vec<String> = map
        .values()
        .map(|item| {
            match &*item.borrow() {
                ProtoItem::Message(message) => &message.name,
                ProtoItem::Enum(en) => &en.name,
            }
            .clone()
        })
        .collect();
    let mut short_counts = HashMap::<&str, usize>::new();
    for original in &originals {
        *short_counts.entry(short_name(original)).or_default() += 1;
    }
    // A short name is usable only when it identifies one type. Nested messages
    // retain their parent path even when the binding supplies a short name.
    let mut local_names: HashMap<String, String> = originals
        .iter()
        .filter_map(|original| {
            names
                .get(original)
                .or_else(|| {
                    (short_counts[short_name(original)] == 1)
                        .then(|| names.get(short_name(original)))
                        .flatten()
                })
                .map(|name| (original.clone(), short_name(name).to_string()))
        })
        .collect();
    loop {
        let mut output_names: HashMap<String, Vec<&String>> = HashMap::new();
        for original in &originals {
            let name = local_names
                .get(original)
                .map(String::as_str)
                .unwrap_or(short_name(original));
            let scope = original
                .rsplit_once('.')
                .map(|(scope, _)| scope)
                .unwrap_or("");
            output_names
                .entry(format!("{scope}.{name}"))
                .or_default()
                .push(original);
        }
        let mut removed = false;
        for originals in output_names.values().filter(|names| names.len() > 1) {
            for original in originals {
                if local_names.remove(*original).is_some() {
                    log::debug!(
                        "NT checkpoint collision for {original}; final decision is deferred"
                    );
                    removed = true;
                }
            }
        }
        if !removed {
            break;
        }
    }

    let mut cmd_ids = HashMap::new();
    let mut nt_map = HashMap::new();
    for original in &originals {
        let mut prefix = String::new();
        let mut renamed = Vec::new();
        for segment in original.split('.') {
            if !prefix.is_empty() {
                prefix.push('.');
            }
            prefix.push_str(segment);
            renamed.push(
                local_names
                    .get(&prefix)
                    .map(String::as_str)
                    .unwrap_or(segment),
            );
        }
        let renamed = renamed.join(".");
        if renamed != *original {
            nt_map.insert(original.clone(), renamed);
        }
    }
    for item in map.values() {
        let mut item = item.borrow_mut();
        match &mut *item {
            ProtoItem::Message(message) => {
                message.deobfuscated_name = nt_map.get(&message.name).cloned();
                if let Some((id, _)) = req_map.get(&message.name) {
                    message.cmd_id = *id;
                    message.msg_type = MessageType::Req;
                } else if let Some(&id) = rsp_notify_map.get(&message.name) {
                    message.cmd_id = id;
                    message.msg_type = if message
                        .deobfuscated_name
                        .as_ref()
                        .is_some_and(|name| name.ends_with("Rsp"))
                        || message.fields.iter().any(|field| {
                            field.name == RETCODE_FIELD_NAME
                                || snake_field(&field.name) == "retcode"
                        }) {
                        MessageType::Rsp
                    } else {
                        MessageType::Notify
                    };
                }
                if message.cmd_id != 0 {
                    cmd_ids.insert(
                        i32::from(message.cmd_id),
                        message
                            .deobfuscated_name
                            .as_ref()
                            .unwrap_or(&message.name)
                            .clone(),
                    );
                }
            }
            ProtoItem::Enum(en) => {
                en.deobfuscated_name = nt_map.get(&en.name).cloned();
            }
        }
    }

    // Resolve all names before rewriting field types, including nested types and maps.
    for item in map.values() {
        if let ProtoItem::Message(message) = &mut *item.borrow_mut() {
            for field in message.fields.iter_mut().chain(
                message
                    .oneofs
                    .iter_mut()
                    .flat_map(|oneof| &mut oneof.fields),
            ) {
                field.kind = rewrite_type_tokens(&field.kind, |name| {
                    nt_map
                        .get(name)
                        .map(String::as_str)
                        .unwrap_or(name)
                        .to_owned()
                });
            }
        }
    }
    (cmd_ids, nt_map)
}

fn csharp_type_to_protobuf_type(
    cache: &TypeCache,
    ty: &RuntimeType,
    number_type: NumberType,
) -> Cow<'static, str> {
    if let Some(ty) = cache.type_map.get(ty) {
        use CachedType::*;
        match ty {
            Boolean => Cow::Borrowed("bool"),
            Int32 => match number_type {
                NumberType::Normal => Cow::Borrowed("sfixed32"),
                NumberType::ZigZagVarint => Cow::Borrowed("sint32"),
                _ => Cow::Borrowed("int32"),
            },
            Int64 => match number_type {
                NumberType::Normal => Cow::Borrowed("sfixed64"),
                NumberType::ZigZagVarint => Cow::Borrowed("sint64"),
                _ => Cow::Borrowed("int64"),
            },
            UInt32 => {
                if let NumberType::Normal = number_type {
                    Cow::Borrowed("fixed32")
                } else {
                    Cow::Borrowed("uint32")
                }
            }
            UInt64 => {
                if let NumberType::Normal = number_type {
                    Cow::Borrowed("fixed64")
                } else {
                    Cow::Borrowed("uint64")
                }
            }
            Single => Cow::Borrowed("float"),
            Double => Cow::Borrowed("double"),
            String => Cow::Borrowed("string"),
            ByteString => Cow::Borrowed("bytes"),
            Any => Cow::Borrowed("google.protobuf.Any"),
            _ => unreachable!(),
        }
    } else {
        let generics = ty.get_generic_arguments();
        match generics.len() {
            1 => Cow::Owned(format!(
                "repeated {}",
                csharp_type_to_protobuf_type(cache, &generics[0], number_type)
            )),
            2 => Cow::Owned(format!(
                "map<{}, {}>",
                csharp_type_to_protobuf_type(cache, &generics[0], number_type),
                csharp_type_to_protobuf_type(cache, &generics[1], number_type)
            )),
            _ => ty.format_type_name(true).into(),
        }
    }
}

pub fn generate_protobuf<W: Write>(
    type_cache: &TypeCache,
    minimal_info_map: &HashMap<RuntimeType, MessageMinimalInfo>,
    rsp_notify_map: &HashMap<RuntimeType, u16>,
    req_map: &HashMap<RuntimeType, (u16, Option<String>)>,
    rsp_notify_names: &HashMap<String, String>,
    method_nt_map: &HashMap<String, String>,
    predeobf_map: &HashMap<String, String>,
    mut out: W,
) -> anyhow::Result<(HashMap<i32, String>, HashMap<String, String>, TypeToItemMap)> {
    let mut type_to_item: TypeToItemMap = IndexMap::new();
    let mut parents = Vec::new();

    for i in unsafe { il2cpp::RPG_NETWORK_PROTO_START..il2cpp::RPG_NETWORK_PROTO_END } {
        let class = metadata_cache::get_typeinfo_from_typedefindex(i);
        let runtime_type = RuntimeType::from_class(class)
            .with_context(|| format!("resolving protocol output type {i}"))?;
        let type_name = runtime_type.get_name()?.as_str();
        if type_name == "<Module>" {
            continue;
        }

        let parent_type = runtime_type.get_declaring_type()?;
        let parent_type = (!parent_type.is_null()).then_some(parent_type);

        let is_nested = parent_type.is_some();

        let proto_item = if let Some(message_info) = minimal_info_map.get(&runtime_type) {
            let mut fields = Vec::<Field>::with_capacity(message_info.fields.len());
            let field_names = super::util::map_field_names_from_property(runtime_type);
            let instance_fields = runtime_type.get_fields_il2cpp();
            let mut numbers = HashSet::new();
            for info in &message_info.fields {
                let number = info.tag >> 3;
                ensure!(
                    number > 0 && numbers.insert(number),
                    "invalid or duplicate field {number} in {type_name}"
                );
            }

            for field_info in message_info
                .fields
                .iter()
                .filter(|f| f.oneof_extra_data.is_none())
            {
                if let Some(property) = field_info.property {
                    fields.push(Field {
                        kind: csharp_type_to_protobuf_type(
                            type_cache,
                            &property.get_property_type()?,
                            field_info.number_type,
                        )
                        .to_string(),
                        name: property.get_name()?.as_str().to_string(),
                        number: field_info.tag >> 3,
                        offset: field_info.offset,
                    });
                } else {
                    let field = instance_fields
                        .iter()
                        .find(|f| f.is_instance() && f.get_offset() as u32 == field_info.offset)
                        .with_context(|| {
                            format!(
                                "{type_name}: no instance field at 0x{:X}",
                                field_info.offset
                            )
                        })?;

                    // 4.6.51 strips most PropertyInfo rows. The native getter has
                    // already identified storage; its field name remains usable.
                    let name = field_names
                        .get(&field.get_offset())
                        .cloned()
                        .unwrap_or(field.get_name()?.as_str());
                    fields.push(Field {
                        kind: csharp_type_to_protobuf_type(
                            type_cache,
                            &field.get_field_type()?,
                            field_info.number_type,
                        )
                        .to_string(),
                        name: name.to_string(),
                        number: field_info.tag >> 3,
                        offset: field_info.offset,
                    });
                }
            }

            fields.sort_by_key(|f| f.number);
            let mut oneofs = Vec::<OneOf>::new();

            for field_info in message_info
                .fields
                .iter()
                .filter(|f| f.oneof_extra_data.is_some())
            {
                if let Some(oneof_variant) = field_info.property {
                    let oneof_item = field_info
                        .oneof_extra_data
                        .as_ref()
                        .unwrap()
                        .property
                        .context("missing oneof discriminator property")?;

                    let oneof = if let Some(oneof) = oneofs
                        .iter_mut()
                        .find(|o| o.name == oneof_item.get_name().unwrap().as_str())
                    {
                        oneof
                    } else {
                        oneofs.push(OneOf {
                            name: oneof_item.get_name().unwrap().as_str().to_string(),
                            fields: Vec::new(),
                        });
                        oneofs.last_mut().unwrap()
                    };

                    oneof.fields.push(Field {
                        kind: csharp_type_to_protobuf_type(
                            type_cache,
                            &oneof_variant.get_property_type().unwrap(),
                            field_info.number_type,
                        )
                        .to_string(),
                        name: oneof_variant.get_name().unwrap().as_str().to_string(),
                        number: field_info.tag >> 3,
                        offset: field_info.offset,
                    });
                } else {
                    let oneof_variant = field_info.oneof_extra_data.as_ref().unwrap();
                    let oneof_enum_field = instance_fields
                        .iter()
                        .find(|f| {
                            f.is_instance()
                                && f.get_offset() as u32 == oneof_variant.oneof_enum_offset
                        })
                        .with_context(|| {
                            format!(
                                "{type_name}: missing oneof discriminator at 0x{:X}",
                                oneof_variant.oneof_enum_offset
                            )
                        })?;
                    let oneof_enum = oneof_enum_field.get_field_type()?;
                    let mut variant_name = None;
                    for field in oneof_enum.get_fields_il2cpp() {
                        if field.get_isliteral()?.unbox()
                            && field.get_value(Il2CppObject::NULL)?.unbox::<i32>() as u32
                                == (field_info.tag >> 3)
                        {
                            variant_name = Some(field.get_name()?.as_str().to_string());
                            break;
                        }
                    }
                    let variant_name = variant_name.with_context(|| {
                        format!("{type_name}: oneof case {} is missing", field_info.tag >> 3)
                    })?;
                    let group_name = field_names
                        .get(&oneof_enum_field.get_offset())
                        .cloned()
                        .unwrap_or(oneof_enum_field.get_name()?.as_str())
                        .to_string();

                    let oneof =
                        if let Some(oneof) = oneofs.iter_mut().find(|o| o.name == group_name) {
                            oneof
                        } else {
                            oneofs.push(OneOf {
                                name: group_name,
                                fields: Vec::new(),
                            });
                            oneofs.last_mut().unwrap()
                        };

                    oneof.fields.push(Field {
                        kind: csharp_type_to_protobuf_type(
                            type_cache,
                            &oneof_variant.variant_type,
                            field_info.number_type,
                        )
                        .to_string(),
                        name: variant_name,
                        number: field_info.tag >> 3,
                        offset: field_info.offset,
                    });
                }
            }

            ProtoItem::Message(Message {
                name: runtime_type.format_type_name(true),
                cmd_id: 0,
                fields,
                oneofs,
                children: Vec::new(),
                has_parent: is_nested,
                deobfuscated_name: None,
                msg_type: MessageType::None,
                write_to_rva: message_info.write_to_rva,
                merge_from_rva: message_info.merge_from_rva,
            })
        } else if matches!(
            type_cache
                .type_map
                .get(&runtime_type.get_base_type().unwrap()),
            Some(&CachedType::Enum)
        ) {
            let mut variants = Vec::new();
            for field in runtime_type.get_fields_il2cpp() {
                if !field.get_isliteral()?.unbox() {
                    continue;
                }
                let mut name = format!("{}_{}", type_name, field.get_name()?.as_str());
                for attr in field.get_custom_attributes() {
                    let attr_type = RuntimeType::from_object(attr)?;
                    if !attr_type.il_name().ends_with(".OriginalNameAttribute") {
                        continue;
                    }
                    for member in attr_type.get_fields_il2cpp() {
                        if member.is_instance()
                            && member.get_field_type()?.il_name() == "System.String"
                            && member.get_name()?.as_str().contains("Name")
                        {
                            let value = Il2CppString(member.get_value(attr)?.0);
                            if !value.is_null() {
                                name = value.as_str().to_string();
                            }
                            break;
                        }
                    }
                }
                variants.push((name, field.get_value(Il2CppObject::NULL)?.unbox::<i32>()));
            }
            variants.sort_by_key(|(_, value)| *value != 0);
            ensure!(
                variants.first().is_some_and(|(_, value)| *value == 0),
                "protobuf enum {type_name} has no zero value"
            );
            ProtoItem::Enum(Enum {
                variants,
                name: runtime_type.format_type_name(true),
                deobfuscated_name: None,
                has_parent: is_nested,
            })
        } else {
            if type_name.starts_with('<') {
                continue;
            }

            ProtoItem::Message(Message {
                name: runtime_type.format_type_name(true),
                cmd_id: 0,
                fields: Vec::new(),
                oneofs: Vec::new(),
                children: Vec::new(),
                has_parent: is_nested,
                deobfuscated_name: None,
                msg_type: MessageType::None,
                write_to_rva: 0,
                merge_from_rva: 0,
            })
        };

        let proto_item = Rc::new(RefCell::new(proto_item));

        type_to_item.insert(runtime_type, proto_item.clone());

        if let Some(parent_rt) = parent_type {
            parents.push((parent_rt, runtime_type, proto_item));
        }
    }

    // Metadata declaration order need not put parents before their children.
    // Resolve ownership after collecting all types so no nested definition is lost.
    for (parent, child, item) in parents {
        let parent_item = type_to_item
            .get(&parent)
            .with_context(|| format!("missing protobuf parent of {}", child.il_name()))?;
        let mut parent_item = parent_item.borrow_mut();
        let ProtoItem::Message(message) = &mut *parent_item else {
            anyhow::bail!("protobuf parent of {} is not a message", child.il_name());
        };
        message.children.push(item);
    }

    let (cmd_ids, nt_map) = finish_protobuf(
        &mut type_to_item,
        &HashMap::new(),
        rsp_notify_map,
        req_map,
        rsp_notify_names,
        method_nt_map,
        predeobf_map,
        &mut out,
    )?;
    Ok((cmd_ids, nt_map, type_to_item))
}

pub(super) fn finish_protobuf<W: Write>(
    type_to_item: &mut TypeToItemMap,
    previous_names: &HashMap<String, String>,
    rsp_notify_map: &HashMap<RuntimeType, u16>,
    req_map: &HashMap<RuntimeType, (u16, Option<String>)>,
    rsp_notify_names: &HashMap<String, String>,
    type_names: &HashMap<String, String>,
    field_names: &HashMap<String, String>,
    mut out: W,
) -> anyhow::Result<(HashMap<i32, String>, HashMap<String, String>)> {
    // Reuse the recovered wire model. Revert the previous names before applying new hints.
    let originals: HashMap<_, _> = previous_names
        .iter()
        .map(|(obf, name)| (name.as_str(), obf.as_str()))
        .collect();
    for item in type_to_item.values() {
        if let ProtoItem::Message(message) = &mut *item.borrow_mut() {
            for field in message.fields.iter_mut().chain(
                message
                    .oneofs
                    .iter_mut()
                    .flat_map(|oneof| &mut oneof.fields),
            ) {
                field.kind = rewrite_type_tokens(&field.kind, |name| {
                    originals.get(name).copied().unwrap_or(name).to_owned()
                });
            }
        }
    }
    let (cmd_ids, nt_map) = process_cmd_id(
        type_to_item,
        &rsp_notify_map
            .iter()
            .map(|(c, cmd_id)| (c.format_type_name(true), *cmd_id))
            .collect::<HashMap<_, _>>(),
        &req_map
            .iter()
            .map(|(c, (cmd_id, deobf))| (c.format_type_name(true), (*cmd_id, deobf.clone())))
            .collect::<HashMap<_, _>>(),
        rsp_notify_names,
        type_names,
    );

    apply_scoped_field_map(type_to_item, field_names);
    validate_output(type_to_item)?;
    let mut rendered = Vec::new();

    writeln!(
        rendered,
        "syntax = \"proto3\"; // ex-RushiaLover ProtoDumper | Game Version: {}\n",
        *crate::version::GAME_VERSION
    )?;
    if type_to_item.values().any(|item| match &*item.borrow() {
        ProtoItem::Message(message) => message
            .fields
            .iter()
            .chain(message.oneofs.iter().flat_map(|oneof| &oneof.fields))
            .any(|field| field.kind.contains("google.protobuf.Any")),
        _ => false,
    }) {
        writeln!(rendered, "import \"google/protobuf/any.proto\";\n")?;
    }

    for proto in type_to_item.values() {
        let proto = proto.borrow();
        match &*proto {
            ProtoItem::Message(msg) => {
                if msg.has_parent {
                    continue;
                }
            }
            ProtoItem::Enum(en) => {
                if en.has_parent {
                    continue;
                }
            }
        }
        writeln!(rendered, "{}", proto.fmt_protobuf_with_depth(0))?;
    }

    super::recovery::validation::validate_source(std::str::from_utf8(&rendered)?)?;
    out.write_all(&rendered)?;

    Ok((cmd_ids, nt_map))
}

pub fn apply_scoped_field_map(type_to_item: &mut TypeToItemMap, map: &HashMap<String, String>) {
    if map.is_empty() {
        return;
    }

    for item in type_to_item.values_mut() {
        let mut item_mut = item.borrow_mut();
        if let ProtoItem::Message(msg) = &mut *item_mut {
            let fields: Vec<_> = msg
                .fields
                .iter_mut()
                .chain(msg.oneofs.iter_mut().flat_map(|oneof| &mut oneof.fields))
                .collect();
            let originals: Vec<_> = fields.iter().map(|field| field.name.clone()).collect();
            let mut proposed: Vec<_> = fields
                .iter()
                .map(|field| {
                    map.get(&super::recovery::field_key(&msg.name, field.number))
                        .unwrap_or(&field.name)
                        .clone()
                })
                .collect();
            loop {
                let mut counts = HashMap::new();
                for name in &proposed {
                    *counts.entry(snake_field(name)).or_insert(0) += 1;
                }
                let mut changed = false;
                for (name, original) in proposed.iter_mut().zip(&originals) {
                    if *name != *original && counts[&snake_field(name)] > 1 {
                        *name = original.clone();
                        changed = true;
                    }
                }
                if !changed {
                    break;
                }
            }
            for (field, name) in fields.into_iter().zip(proposed) {
                field.name = name;
            }
        }
    }
}

fn validate_output(items: &TypeToItemMap) -> anyhow::Result<()> {
    let mut defined = HashSet::new();
    for item in items.values() {
        let item = item.borrow();
        let name = match &*item {
            ProtoItem::Message(message) => {
                message.deobfuscated_name.as_ref().unwrap_or(&message.name)
            }
            ProtoItem::Enum(en) => en.deobfuscated_name.as_ref().unwrap_or(&en.name),
        };
        ensure!(
            defined.insert(name.clone()),
            "duplicate protobuf definition {name}"
        );
    }
    let scalar = [
        "repeated",
        "map",
        "bool",
        "int32",
        "int64",
        "uint32",
        "uint64",
        "sint32",
        "sint64",
        "fixed32",
        "fixed64",
        "sfixed32",
        "sfixed64",
        "float",
        "double",
        "string",
        "bytes",
        "google.protobuf.Any",
    ];
    for item in items.values() {
        let item = item.borrow();
        if let ProtoItem::Message(message) = &*item {
            let mut names = HashSet::new();
            for field in message
                .fields
                .iter()
                .chain(message.oneofs.iter().flat_map(|oneof| &oneof.fields))
            {
                let name = snake_field(&field.name);
                ensure!(
                    !name.is_empty() && names.insert(name),
                    "duplicate or empty field name in {}",
                    message.name
                );
                for token in field
                    .kind
                    .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '.'))
                    .filter(|part| !part.is_empty())
                {
                    ensure!(
                        scalar.contains(&token) || defined.contains(token),
                        "{} field {} references missing protobuf type {token}",
                        message.name,
                        field.number
                    );
                }
            }
        }
    }
    Ok(())
}
