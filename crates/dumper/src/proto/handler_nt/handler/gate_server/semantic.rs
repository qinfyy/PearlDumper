use iced_x86::Register;
use il2cpp::vm::object::Il2CppObject;
use reflection::{method_info::MethodInfo, runtime_type::RuntimeType};

use crate::proto::{
    output::Message,
    recovery::{
        Candidate, Evidence, Hints, field_key,
        flow::{ParserLayout, field_copies},
    },
    type_references::TypeReferences,
};

pub(super) fn code(rva: usize, limit: usize) -> Option<&'static [u8]> {
    let table = utils::scanner::game_assembly_functions().ok()?;
    let range = table.containing(rva);
    if range.is_some_and(|r| r.start as usize != rva) {
        return None;
    }
    utils::scanner::game_assembly_code_sections().find_map(|(start, bytes)| {
        let offset = rva.checked_sub(start)?;
        let size = range.map_or(limit, |r| (r.end as usize - rva).min(limit));
        bytes.get(offset..offset.checked_add(size)?.min(bytes.len()))
    })
}

pub(super) fn recover_ip(
    message: &Message,
    game: RuntimeType,
    parser: &MethodInfo,
    bytes: &[u8],
    rva: usize,
    references: &TypeReferences,
    allocation: usize,
) -> Hints {
    let mut hints = Hints::new();
    let result = (|| -> anyhow::Result<()> {
        let address_fields: Vec<_> = game
            .get_fields_il2cpp()
            .into_iter()
            .filter(|f| {
                f.is_instance()
                    && f.get_name()
                        .is_ok_and(|n| n.as_str() == "GateServerAddress")
            })
            .collect();
        let [address] = address_fields.as_slice() else {
            anyhow::bail!("address field not unique")
        };
        let address_type = address.get_field_type()?;
        let selectors: Vec<_> = address_type
            .get_methods_il2cpp()
            .into_iter()
            .filter_map(|m| {
                if m.is_static()
                    || !m
                        .get_return_type()
                        .is_ok_and(|t| t.il_name() == "System.String")
                {
                    return None;
                }
                let parameters = m.get_parameters();
                let [p] = parameters.as_slice() else {
                    return None;
                };
                let ty = p.get_parameter_type().ok()?;
                (ty.il_name() == "RPG.Client.IPVersion").then_some((m, ty))
            })
            .collect();
        let [(selector, ip_version)] = selectors.as_slice() else {
            anyhow::bail!("IPVersion string selector not unique")
        };
        let enum_fields: Vec<_> = ip_version
            .get_fields_il2cpp()
            .into_iter()
            .filter(|f| {
                f.get_isliteral().is_ok_and(|v| v.unbox())
                    && f.get_name().is_ok_and(|n| n.as_str() == "IPv4")
            })
            .collect();
        let [ipv4] = enum_fields.as_slice() else {
            anyhow::bail!("IPv4 enum constant missing or ambiguous")
        };
        let value = ipv4.get_value(Il2CppObject::NULL)?.unbox::<i32>();
        let selector_rva = selector.get_il2cpp_method().checked_rva()?;
        let selector_bytes =
            code(selector_rva, 128).ok_or_else(|| anyhow::anyhow!("selector is not executable"))?;
        let offset =
            super::ipv4::selected_field(selector_bytes, *il2cpp::GA_BASE + selector_rva, value)
                .ok_or_else(|| anyhow::anyhow!("unsupported IPv4 selector"))?;
        anyhow::ensure!(
            address_type.get_fields_il2cpp().iter().any(|f| {
                f.is_instance()
                    && f.get_offset() == offset
                    && f.get_field_type()
                        .is_ok_and(|t| t.il_name() == "System.String")
            }),
            "IPv4 selector did not return string storage"
        );
        let copies = field_copies(
            bytes,
            *il2cpp::GA_BASE + rva,
            ParserLayout {
                proto_argument: if parser.is_static() {
                    Register::RCX
                } else {
                    Register::RDX
                },
                game_argument: None,
                allocation: *il2cpp::GA_BASE + allocation,
                game_class: address_type.get_il2cpp_type().get_class().0,
            },
            |slot| references.class_from_slot(slot).map(|c| c.0),
        )?;
        for copy in copies
            .iter()
            .filter(|c| c.game_offset as usize == offset && c.width == 8)
        {
            let fields: Vec<_> = message
                .fields
                .iter()
                .chain(message.oneofs.iter().flat_map(|o| &o.fields))
                .filter(|f| f.offset == copy.proto_offset)
                .collect();
            let [field] = fields.as_slice() else { continue };
            if field.kind != "string" {
                continue;
            }
            hints.entries.push(Candidate {
                symbol: field_key(&message.name, field.number), name: "ip".into(),
                original_name: Some(field.name.clone()),
                source: format!("GateServerAddress type allocation and native copy; {}::{}(IPv4={value}) RVA 0x{selector_rva:X} returns string slot 0x{offset:X}", address_type.il_name(), selector.get_name()?.as_str()),
                method: Some(format!("ServerDispatchData::_ParseServerDispatchData; copy at RVA 0x{:X}", copy.instruction - *il2cpp::GA_BASE)),
                rva: Some(rva), evidence: Evidence::Native,
            });
        }
        Ok(())
    })();
    if let Err(error) = result {
        let mut rejected = crate::proto::recovery::candidate(
            &message.name,
            "",
            &format!("GateServer IPv4 recovery skipped: {error:#}"),
            Evidence::Hint,
        );
        rejected.rva = Some(rva);
        hints.entries.push(rejected);
    }
    hints
}

fn runtime_type(name: &str) -> anyhow::Result<RuntimeType> {
    RuntimeType::from_class(
        il2cpp::get_cached_class(name).ok_or_else(|| anyhow::anyhow!("missing {name}"))?,
    )
}

fn named_offset(ty: RuntimeType, name: &str, kind: &str) -> anyhow::Result<u32> {
    let fields: Vec<_> = ty
        .get_fields_il2cpp()
        .into_iter()
        .filter(|f| {
            f.is_instance()
                && f.get_name().is_ok_and(|n| n.as_str() == name)
                && f.get_field_type().is_ok_and(|t| t.il_name() == kind)
        })
        .collect();
    let [field] = fields.as_slice() else {
        anyhow::bail!("missing/ambiguous {name} storage")
    };
    Ok(u32::try_from(field.get_offset())?)
}

fn conversion(ty: RuntimeType, name: &str, args: &[&str]) -> anyhow::Result<usize> {
    let methods: Vec<_> = ty
        .get_methods_il2cpp()
        .into_iter()
        .filter(|m| {
            m.is_static()
                && m.get_name().is_ok_and(|n| n.as_str() == name)
                && m.get_return_type()
                    .is_ok_and(|t| t.il_name() == "System.Byte[]")
                && m.get_parameters()
                    .iter()
                    .map(|p| p.get_parameter_type().map(|t| t.il_name().into_owned()))
                    .collect::<anyhow::Result<Vec<_>>>()
                    .is_ok_and(|p| p == args)
        })
        .collect();
    let [method] = methods.as_slice() else {
        anyhow::bail!("missing/ambiguous Convert::{name}")
    };
    Ok(*il2cpp::GA_BASE + method.get_il2cpp_method().checked_rva()?)
}

pub(super) fn recover_secret(
    message: &Message,
    proto: RuntimeType,
    game: RuntimeType,
    parser: &MethodInfo,
) -> Hints {
    let mut hints = Hints::new();
    let result = (|| -> anyhow::Result<()> {
        let manager = runtime_type("RPG.Client.NetworkManager")?;
        let convert = runtime_type("System.Convert")?;
        let string = runtime_type("System.String")?;
        let fields: Vec<_> = manager
            .get_fields_il2cpp()
            .into_iter()
            .filter(|f| {
                f.is_instance()
                    && f.get_field_type()
                        .is_ok_and(|t| t.il_name() == "System.Byte[]")
            })
            .collect();
        let [key] = fields.as_slice() else {
            anyhow::bail!("NetworkManager byte[] storage is not unique")
        };
        let key_offset = u32::try_from(key.get_offset())?;
        let parser_address = *il2cpp::GA_BASE + parser.get_il2cpp_method().checked_rva()?;
        let mut parsers = vec![(
            parser_address,
            if parser.is_static() {
                Register::RCX
            } else {
                Register::RDX
            },
        )];
        let methods = manager.get_methods_il2cpp();
        // Wrappers can be renamed; discover them by their live signatures and
        // actual call to the already identified ServerDispatchData parser.
        for m in &methods {
            if !m.get_return_type().is_ok_and(|t| t == game) {
                continue;
            }
            let args = m.get_parameters();
            let [p] = args.as_slice() else { continue };
            if !p.get_parameter_type().is_ok_and(|t| t == proto) {
                continue;
            }
            let Ok(rva) = m.get_il2cpp_method().checked_rva() else {
                continue;
            };
            let Some(bytes) = complete_code(rva) else {
                continue;
            };
            if iced_x86::Decoder::with_ip(
                64,
                bytes,
                (*il2cpp::GA_BASE + rva) as u64,
                iced_x86::DecoderOptions::NONE,
            )
            .into_iter()
            .any(|i| crate::proto::native::direct_target(&i) == Some(parser_address))
            {
                parsers.push((
                    *il2cpp::GA_BASE + rva,
                    if m.is_static() {
                        Register::RCX
                    } else {
                        Register::RDX
                    },
                ));
            }
        }
        let layout = super::secret::Layout {
            parsers,
            base64_chars: conversion(
                convert,
                "FromBase64CharPtr",
                &["System.Char*", "System.Int32"],
            )?,
            base64_string: conversion(convert, "FromBase64String", &["System.String"])?,
            string_length: named_offset(string, "m_stringLength", "System.Int32")?,
            string_chars: named_offset(string, "m_firstChar", "System.Char")?,
            key_offset,
        };
        for m in &methods {
            if m.is_static()
                || !m
                    .get_return_type()
                    .is_ok_and(|t| t.il_name() == "System.Boolean")
                || !m.get_parameters().first().is_some_and(|p| {
                    p.get_parameter_type()
                        .is_ok_and(|t| t.il_name() == "System.String")
                })
            {
                continue;
            }
            let Ok(rva) = m.get_il2cpp_method().checked_rva() else {
                continue;
            };
            let Some(bytes) = complete_code(rva) else {
                continue;
            };
            let Ok(observations) = super::secret::recover(bytes, *il2cpp::GA_BASE + rva, &layout)
            else {
                continue;
            };
            for observation in observations {
                let fields: Vec<_> = message
                    .fields
                    .iter()
                    .chain(message.oneofs.iter().flat_map(|o| &o.fields))
                    .filter(|f| f.offset == observation.offset)
                    .collect();
                let [field] = fields.as_slice() else { continue };
                if field.kind != "string" {
                    continue;
                }
                hints.entries.push(Candidate {
                    symbol: field_key(&message.name, field.number), name: "client_secret_key".into(),
                    original_name: Some(field.name.clone()), evidence: Evidence::Native, rva: Some(rva),
                    method: Some(format!("RPG.Client.NetworkManager::{}; store at RVA 0x{:X}", m.get_name()?.as_str(), observation.store - *il2cpp::GA_BASE)),
                    source: format!("GateServer parser argument identity; Base64 decode RVA 0x{:X} -> native key transform RVA 0x{:X} -> NetworkManager byte[] slot 0x{key_offset:X}", observation.decode - *il2cpp::GA_BASE, observation.transform - *il2cpp::GA_BASE),
                });
            }
        }
        anyhow::ensure!(
            !hints.is_empty(),
            "no proven GateServer Base64/native-key storage chain"
        );
        Ok(())
    })();
    if let Err(error) = result {
        hints.entries.push(crate::proto::recovery::candidate(
            &message.name,
            "",
            &format!("GateServer secret recovery skipped: {error:#}"),
            Evidence::Hint,
        ));
    }
    hints
}

fn complete_code(rva: usize) -> Option<&'static [u8]> {
    let table = utils::scanner::game_assembly_functions().ok()?;
    let range = table.containing(rva)?;
    (range.start as usize == rva).then_some(())?;
    utils::scanner::game_assembly_function(rva).ok()
}
