use iced_x86::{
    Decoder, DecoderOptions, InstructionInfoFactory, Mnemonic, OpAccess, OpKind, Register,
};
use il2cpp::vm::{object::Il2CppObject, value::Il2CppValue};
use indexmap::IndexMap;
use reflection::{field_info::FieldInfo, runtime_type::RuntimeType};
use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
};

use super::{FieldMinimalInfo, MessageMinimalInfo, NumberType, cache::TypeCache};

// VarInt encoding helpers.
pub fn varint_length(mut v: u32) -> usize {
    if v == 0 {
        return 1;
    }

    let mut logcounter = 0;
    while v > 0 {
        logcounter += 1;
        v >>= 7;
    }
    logcounter
}

pub fn encode_varint(dst: &mut Vec<u8>, value: u32) -> usize {
    const MSB: u8 = 0b1000_0000;

    let mut n = value;
    let mut i = 0;

    while n >= 0x80 {
        dst.push(MSB | (n as u8));
        i += 1;
        n >>= 7;
    }

    dst.push(n as u8);
    i + 1
}

// Wire types. 3 and 4 are deprecated and hoyo don't use them (SGROUP and EGROUP)
pub const WIRE_TYPE_VAR_INT: u8 = 0;
pub const WIRE_TYPE_I64: u8 = 1;
pub const WIRE_TYPE_LENGTH_PREFIXED: u8 = 2;
pub const WIRE_TYPE_I32: u8 = 5;

pub fn pack_wire_tag(field_id: u32, wire_type: u8) -> u32 {
    (field_id << 3) | (wire_type as u32)
}

pub fn is_correct_getter(field_offset: usize, getter_rva: usize) -> bool {
    let Ok((bytes, _)) = super::fields::code(getter_rva) else {
        return false;
    };
    let mut owners = HashSet::from([Register::RCX]);
    let mut info = InstructionInfoFactory::new();
    for instruction in Decoder::new(64, &bytes[..bytes.len().min(1024)], DecoderOptions::NONE) {
        if instruction.is_invalid()
            || matches!(instruction.mnemonic(), Mnemonic::Ret | Mnemonic::Int3)
        {
            break;
        }
        if matches!(
            instruction.mnemonic(),
            Mnemonic::Mov
                | Mnemonic::Movzx
                | Mnemonic::Movsx
                | Mnemonic::Movsxd
                | Mnemonic::Movss
                | Mnemonic::Movsd
        ) && instruction.op0_kind() == OpKind::Register
            && instruction.op1_kind() == OpKind::Memory
            && owners.contains(&instruction.memory_base().full_register())
            && instruction.memory_index() == Register::None
            && instruction.memory_displacement64() as usize == field_offset
        {
            return true;
        }
        // A prologue MOV or a global load is not an instance field access.
        let copy = instruction.mnemonic() == Mnemonic::Mov
            && instruction.op0_kind() == OpKind::Register
            && instruction.op1_kind() == OpKind::Register
            && owners.contains(&instruction.op1_register().full_register());
        for used in info.info(&instruction).used_registers() {
            if matches!(
                used.access(),
                OpAccess::Write
                    | OpAccess::CondWrite
                    | OpAccess::ReadWrite
                    | OpAccess::ReadCondWrite
            ) {
                owners.remove(&used.register().full_register());
            }
        }
        if copy {
            owners.insert(instruction.op0_register().full_register());
        }
        if instruction.mnemonic() == Mnemonic::Call {
            for reg in [
                Register::RAX,
                Register::RCX,
                Register::RDX,
                Register::R8,
                Register::R9,
                Register::R10,
                Register::R11,
            ] {
                owners.remove(&reg);
            }
        }
    }

    false
}

pub fn map_field_names_from_property(ty: RuntimeType) -> IndexMap<usize, Cow<'static, str>> {
    let getter_methods = ty
        .get_properties(60)
        .into_iter()
        .filter_map(|p| {
            let get_method = p.get_get_method(true).ok()?;
            if !get_method.is_null() {
                Some((p, get_method))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    ty.get_fields_il2cpp()
        .into_iter()
        .filter(|field| field.is_instance())
        .filter_map(|f| {
            getter_methods
                .iter()
                .find(|(_, m)| {
                    m.get_il2cpp_method()
                        .checked_rva()
                        .is_ok_and(|rva| is_correct_getter(f.get_offset(), rva))
                })
                .and_then(|(p, _)| {
                    p.get_name()
                        .ok()
                        .map(|name| (f.get_offset(), name.as_str()))
                })
        })
        .collect()
}

pub fn generate_minimal_info_from_constants(
    runtime_type: RuntimeType,
    message_info: &mut MessageMinimalInfo,
    _: &TypeCache,
) {
    let fields = runtime_type.get_fields(24); // Public | Static
    let mut field_numbers = Vec::with_capacity(fields.len());

    for field in fields {
        if field.get_isliteral().unwrap().unbox() {
            field_numbers.push(field.get_value(Il2CppObject::NULL).unwrap().unbox::<i32>());
        }
    }

    let mut members = Vec::new();
    let mut oneofs = HashMap::new();

    let properties = runtime_type.get_properties(20); // Public | Instance
    let instance_fields = runtime_type.get_fields(62); // Public | NonPublic | Instance

    for (i, property) in properties.into_iter().enumerate() {
        let field_number = field_numbers.get(i);
        let offset = property
            .get_get_method(true)
            .ok()
            .and_then(|get_method| {
                if !get_method.is_null() {
                    instance_fields.iter().find_map(|f| {
                        if is_correct_getter(
                            f.get_il2cpp_field().offset(),
                            get_method.get_il2cpp_method().rva(),
                        ) {
                            Some(f.get_il2cpp_field().offset() as u32)
                        } else {
                            None
                        }
                    })
                } else {
                    None
                }
            })
            .unwrap_or(0);

        if let Some(field_number) = field_number {
            members.push((field_number, property, offset));
        } else {
            let property_type = property.get_property_type().unwrap();
            if property_type.get_isenum().unwrap().unbox() {
                let enum_fields = property_type.get_fields(24);
                for enum_field in enum_fields {
                    let enum_value = enum_field
                        .get_value(Il2CppObject::NULL)
                        .unwrap()
                        .unbox::<i32>();
                    if enum_value == 0 {
                        continue;
                    }

                    oneofs.insert(enum_value, (property, offset));
                }
            }
        }
    }

    for (field_number, property, offset) in members.into_iter() {
        let is_oneof = oneofs.contains_key(field_number);
        message_info.fields.push(FieldMinimalInfo {
            number_type: NumberType::None,
            offset: if is_oneof { 0 } else { offset },
            oneof_extra_data: oneofs.get(field_number).map(|(p, enum_offset)| {
                super::OneofVariantInfo {
                    oneof_enum_offset: *enum_offset,
                    variant_type: p.get_property_type().unwrap(),
                    property: Some(*p),
                }
            }),
            tag: (*field_number as u32) << 3,
            xor: 0,
            property: Some(property),
        });
    }
}

pub fn map_oneof_enum_getter(fields: &mut Vec<FieldInfo>) -> (Vec<usize>, Vec<(usize, i32)>) {
    // skip if doesn't have "System.Object" / oneof setter
    if !fields
        .iter()
        .any(|v| v.get_field_type().unwrap().il_name() == "System.Object")
    {
        return (Vec::new(), Vec::new());
    }

    let mut oneof_fields = Vec::new();
    let mut enum_fields = Vec::new();

    fields.retain(|v| {
        let typ = v.get_field_type().unwrap();
        let typ_string = typ.get_full_name().unwrap().as_str();

        // this is oneof setter (actual value)
        if typ_string == "System.Object" {
            oneof_fields.push(v.get_offset());
            false
            // TODO might be a problem in future
            // oneofs will have odd dots
        } else if typ.get_isenum().unwrap().unbox() && typ_string.matches('+').count() % 2 != 0 {
            enum_fields.push((v.get_offset(), (*v).get_field_type().unwrap()));
            false
        } else {
            true
        }
    });

    (
        oneof_fields,
        enum_fields
            .iter()
            .flat_map(|(enum_offset, runtime_type)| {
                runtime_type
                    .get_fields_il2cpp()
                    .iter()
                    .filter_map(|f| {
                        if f.get_name().unwrap().as_str() == "value__" {
                            return None;
                        }
                        Some((
                            *enum_offset,
                            f.get_value(Il2CppObject::NULL).unwrap().unbox::<i32>(),
                        ))
                    })
                    .filter(|(_, v)| *v != 0) // exclude enum with zero value
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>(),
    )
}

pub fn is_obf(s: &str) -> bool {
    s.len() == 11 && s.chars().all(char::is_uppercase)
}
