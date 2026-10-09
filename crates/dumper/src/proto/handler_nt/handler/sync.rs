use std::collections::HashMap;

use iced_x86::Register;
use reflection::runtime_type::RuntimeType;

use crate::proto::{
    output::{ProtoItem, TypeToItemMap},
    recovery::{
        Evidence, Hints, candidate, field_key,
        flow::{ParserLayout, field_copies},
    },
};

pub(super) fn process(items: &TypeToItemMap, class_name: &str, method_name: &str) -> Hints {
    let mut hints = Hints::new();
    let Some(owner) =
        il2cpp::get_cached_class(class_name).and_then(|c| RuntimeType::from_class(c).ok())
    else {
        return hints;
    };
    let methods: Vec<_> = owner
        .get_methods_il2cpp()
        .into_iter()
        .filter_map(|m| {
            if m.is_static()
                || !m.get_name().is_ok_and(|n| n.as_str() == method_name)
                || !m
                    .get_return_type()
                    .is_ok_and(|ty| ty.il_name() == "System.Void")
            {
                return None;
            }
            let parameters = m.get_parameters();
            let [parameter] = parameters.as_slice() else {
                return None;
            };
            let ty = parameter.get_parameter_type().ok()?;
            items.contains_key(&ty).then_some((m, ty))
        })
        .collect();
    let [(method, proto)] = methods.as_slice() else {
        return hints;
    };
    let item = items[proto].borrow();
    let ProtoItem::Message(message) = &*item else {
        return hints;
    };
    let Ok(rva) = method.get_il2cpp_method().checked_rva() else {
        return hints;
    };
    let method_name = format!("{class_name}::{method_name}");
    let result = (|| -> anyhow::Result<()> {
        let table = utils::scanner::game_assembly_functions()?;
        anyhow::ensure!(
            table
                .containing(rva)
                .is_some_and(|range| range.start as usize == rva),
            "not a complete PE function entry"
        );
        let bytes = utils::scanner::game_assembly_function(rva)?;
        let copies = field_copies(
            bytes,
            *il2cpp::GA_BASE + rva,
            ParserLayout {
                proto_argument: Register::RDX,
                game_argument: Some(Register::RCX),
                allocation: 0,
                game_class: 0,
            },
            |_| None,
        )?;
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
        let names = super::build_game_offset_candidates(owner);
        for copy in copies {
            let Some(keys) = fields.get(&copy.proto_offset) else {
                continue;
            };
            // A shared oneof storage slot alone does not identify its active member.
            let [key] = keys.as_slice() else { continue };
            if let Some(names) = names.get(&(copy.game_offset as usize)) {
                for name in names {
                    name.submit(&mut hints, key, &method_name, rva, &copy);
                }
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        let mut rejected = candidate(
            &message.name,
            "",
            &format!("native sync rejected: {error:#}"),
            Evidence::Hint,
        );
        rejected.method = Some(method_name);
        rejected.rva = Some(rva);
        hints.entries.push(rejected);
    }
    hints
}
