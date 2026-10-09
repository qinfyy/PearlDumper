use crate::proto::recovery::Hints;
#[cfg(feature = "gateway-network")]
pub mod decode_gateway;
pub mod decoder;
#[cfg(feature = "gateway-network")]
pub mod dispatch;
mod ipv4;
mod names;
pub mod parse_gate_server;
pub mod region;
mod secret;
mod semantic;
pub mod stop_info;

use std::collections::HashMap;

use iced_x86::Register;

use crate::proto::output::TypeToItemMap;

pub fn process_all(type_to_item: &TypeToItemMap) -> Hints {
    let mut map = parse_gate_server::process(type_to_item);
    map.extend(stop_info::process_stop_info(type_to_item));
    #[cfg(feature = "gateway-network")]
    {
        let decode_nt = decode_gateway::run();
        map.extend(decode_nt);
    }
    map
}

fn recover_parser(items: &TypeToItemMap, method_name: &str, game_class_name: &str) -> Hints {
    use crate::proto::{
        recovery::{
            field_key,
            flow::{ParserLayout, field_copies},
        },
        type_references::TypeReferences,
    };
    use reflection::runtime_type::RuntimeType;
    let mut hints = Hints::new();
    let Some(owner) = il2cpp::get_cached_class("RPG.Client.ServerDispatchData")
        .and_then(|c| RuntimeType::from_class(c).ok())
    else {
        return hints;
    };
    let methods: Vec<_> = owner
        .get_methods_il2cpp()
        .into_iter()
        .filter(|m| m.get_name().is_ok_and(|n| n.as_str() == method_name))
        .collect();
    let [method] = methods.as_slice() else {
        return hints;
    };
    let parameters = method.get_parameters();
    let [parameter] = parameters.as_slice() else {
        return hints;
    };
    let Ok(proto) = parameter.get_parameter_type() else {
        return hints;
    };
    let Some(game) = il2cpp::get_cached_class(game_class_name) else {
        return hints;
    };
    let Ok(game_type) = RuntimeType::from_class(game) else {
        return hints;
    };
    if !method.get_return_type().is_ok_and(|ty| ty == game_type) {
        return hints;
    }
    let Some(item) = items.get(&proto) else {
        return hints;
    };
    let item = item.borrow();
    let crate::proto::output::ProtoItem::Message(message) = &*item else {
        return hints;
    };
    let mut fields = HashMap::<u32, Vec<String>>::new();
    for f in message
        .fields
        .iter()
        .chain(message.oneofs.iter().flat_map(|o| &o.fields))
    {
        fields
            .entry(f.offset)
            .or_default()
            .push(field_key(&message.name, f.number));
    }
    #[cfg(feature = "gateway-network")]
    if method_name == "_ParseServerDispatchData" {
        decode_gateway::set_proto_fields(
            message
                .fields
                .iter()
                .map(|f| (f.number, field_key(&message.name, f.number)))
                .collect(),
        );
    }
    let Ok(rva) = method.get_il2cpp_method().checked_rva() else {
        return hints;
    };
    if !utils::scanner::game_assembly_functions()
        .ok()
        .and_then(|table| table.containing(rva))
        .is_some_and(|range| range.start as usize == rva)
    {
        hints.entries.push(crate::proto::recovery::candidate(
            &message.name,
            "",
            &format!("{method_name} RVA 0x{rva:X}: not a complete PE function entry"),
            crate::proto::recovery::Evidence::Hint,
        ));
        return hints;
    }
    let Ok(bytes) = utils::scanner::game_assembly_function(rva) else {
        return hints;
    };
    let Ok(references) = TypeReferences::from_runtime() else {
        return hints;
    };
    let Some(&allocation) = crate::proto::IL2CPP_OBJECT_NEW_RVA.get() else {
        return hints;
    };
    let game_fields = super::build_game_offset_candidates(game_type);
    let layout = ParserLayout {
        proto_argument: if method.is_static() {
            Register::RCX
        } else {
            Register::RDX
        },
        game_argument: (!method.is_static() && owner == game_type).then_some(Register::RCX),
        allocation: *il2cpp::GA_BASE + allocation,
        game_class: game.0,
    };
    match field_copies(bytes, *il2cpp::GA_BASE + rva, layout, |slot| {
        references.class_from_slot(slot).map(|c| c.0)
    }) {
        Ok(copies) => {
            for copy in copies {
                if let (Some(keys), Some(names)) = (
                    fields.get(&copy.proto_offset),
                    game_fields.get(&(copy.game_offset as usize)),
                ) {
                    let [key] = keys.as_slice() else { continue };
                    for name in names {
                        name.submit(
                            &mut hints,
                            key,
                            &format!("{}::{method_name}", owner.il_name()),
                            rva,
                            &copy,
                        );
                    }
                }
            }
        }
        Err(error) => {
            hints.entries.push(crate::proto::recovery::candidate(
                &message.name,
                "",
                &format!("{method_name} RVA 0x{rva:X}: {error:#}"),
                crate::proto::recovery::Evidence::Hint,
            ));
        }
    }
    if method_name == "_ParseServerDispatchData" {
        let mut identity = crate::proto::recovery::candidate(
            &message.name,
            "GateServer",
            "ServerDispatchData parser: exact parameter and return signature; complete native body",
            crate::proto::recovery::Evidence::Native,
        );
        identity.method = Some(format!("{}::{method_name}", owner.il_name()));
        identity.rva = Some(rva);
        hints.entries.push(identity);
        names::normalize(&mut hints, |key| {
            message
                .fields
                .iter()
                .chain(message.oneofs.iter().flat_map(|o| &o.fields))
                .find(|f| field_key(&message.name, f.number) == key)
                .map(|f| f.kind.clone())
        });
        hints.extend(semantic::recover_ip(
            &message,
            game_type,
            method,
            bytes,
            rva,
            &references,
            allocation,
        ));
        hints.extend(semantic::recover_secret(&message, proto, game_type, method));
    }
    hints
}

pub fn full_reg(r: Register) -> Register {
    match r {
        Register::EAX | Register::AX | Register::AL | Register::AH => Register::RAX,
        Register::EBX | Register::BX | Register::BL | Register::BH => Register::RBX,
        Register::ECX | Register::CX | Register::CL | Register::CH => Register::RCX,
        Register::EDX | Register::DX | Register::DL | Register::DH => Register::RDX,
        Register::ESI | Register::SI => Register::RSI,
        Register::EDI | Register::DI => Register::RDI,
        Register::EBP | Register::BP => Register::RBP,
        Register::R8D | Register::R8W | Register::R8L => Register::R8,
        Register::R9D | Register::R9W | Register::R9L => Register::R9,
        Register::R10D | Register::R10W | Register::R10L => Register::R10,
        Register::R11D | Register::R11W | Register::R11L => Register::R11,
        Register::R12D | Register::R12W | Register::R12L => Register::R12,
        Register::R13D | Register::R13W | Register::R13L => Register::R13,
        Register::R14D | Register::R14W | Register::R14L => Register::R14,
        Register::R15D | Register::R15W | Register::R15L => Register::R15,
        _ => r,
    }
}

pub const VOLATILE_REGS: [Register; 7] = [
    Register::RAX,
    Register::RCX,
    Register::RDX,
    Register::R8,
    Register::R9,
    Register::R10,
    Register::R11,
];
