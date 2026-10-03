use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use anyhow::{Context, ensure};
use iced_x86::{
    Decoder, DecoderOptions, Instruction, InstructionInfoFactory, Mnemonic, OpAccess, OpKind,
    Register,
};
use il2cpp::{
    api, get_native_method,
    vm::{method::Il2CppMethod, object::Il2CppObject},
};
use reflection::runtime_type::RuntimeType;
use utils::{game_assembly_slice, scanner::game_assembly_function};

use super::{FieldMinimalInfo, MessageMinimalInfo, NumberType, OneofVariantInfo, native};

struct Getter {
    method: Il2CppMethod,
    ty: RuntimeType,
    reads: BTreeSet<usize>,
    cases: BTreeMap<usize, u32>,
    objects: Vec<usize>,
}

pub(super) struct FieldDecoder {
    raw_tags: HashMap<usize, usize>,
    varint_writers: HashSet<usize>,
    sint_writers: HashSet<usize>,
}

impl FieldDecoder {
    pub fn new() -> anyhow::Result<Self> {
        let mut raw_tags = HashMap::new();
        for count in 1..=3 {
            let signature = format!(
                "Google.Protobuf.CodedOutputStream::WriteRawTag({})",
                vec!["System.Byte"; count].join(",")
            );
            let method =
                get_native_method(&signature).with_context(|| format!("missing {signature}"))?;
            raw_tags.insert(method.checked_va()?, count);
        }
        let mut varint_writers = HashSet::new();
        let mut sint_writers = HashSet::new();
        for (name, argument) in [
            ("WriteRawVarint32", "System.UInt32"),
            ("WriteRawVarint64", "System.UInt64"),
            ("WriteSInt32", "System.Int32"),
            ("WriteSInt64", "System.Int64"),
        ] {
            if let Some(method) = get_native_method(&format!(
                "Google.Protobuf.CodedOutputStream::{name}({argument})"
            )) {
                let address = method.checked_va()?;
                varint_writers.insert(address);
                if name.starts_with("WriteSInt") {
                    sint_writers.insert(address);
                }
            }
        }
        Ok(Self {
            raw_tags,
            varint_writers,
            sint_writers,
        })
    }

    pub fn recover(&self, ty: RuntimeType, output: &mut MessageMinimalInfo) -> anyhow::Result<()> {
        let mut fields = ty.get_fields_il2cpp();
        fields.sort_by_key(|field| field.get_metadata_token());
        let mut numbers = Vec::new();
        let mut storage = HashMap::new();
        for field in fields {
            if field.get_isliteral()?.unbox() && field.get_field_type()?.il_name() == "System.Int32"
            {
                let number = field.get_value(Il2CppObject::NULL)?.unbox::<i32>();
                ensure!(
                    (1..=0x1fff_ffff).contains(&number),
                    "invalid field number {number}"
                );
                numbers.push(number as u32);
            } else if field.is_instance()
                && field.get_field_type()?.il_name() != super::UNKNOWN_FIELD_SET
            {
                storage.insert(field.get_offset(), field.get_field_type()?);
            }
        }
        ensure!(
            numbers.iter().collect::<HashSet<_>>().len() == numbers.len(),
            "duplicate protobuf field constants"
        );
        let mut getters = Vec::new();
        for method in ty.get_il2cpp_type().get_class().get_methods() {
            let name = method.get_name();
            if !(name.starts_with("get_") || super::util::is_obf(&name))
                || api::il2cpp_method_get_param_count(method) != 0
            {
                continue;
            }
            let return_type = api::il2cpp_method_get_return_type(method);
            if return_type.il_name() == "System.Void" {
                continue;
            }
            if let Some(getter) = read_getter(
                method,
                RuntimeType::from_il2cpp_type(return_type)?,
                &storage,
            )? {
                getters.push(getter);
            }
        }
        let discriminators: HashSet<usize> = getters
            .iter()
            .filter(|getter| !getter.objects.is_empty())
            .flat_map(|getter| getter.cases.keys().copied())
            .collect();
        getters.retain(|getter| {
            !(getter.reads.len() == 1 && discriminators.contains(getter.reads.first().unwrap()))
        });
        ensure!(
            getters.len() == numbers.len(),
            "stripped getter coverage mismatch: {} getters, {} field constants",
            getters.len(),
            numbers.len()
        );

        let (tags, zigzag) = self.wire_types(output.write_to_rva)?;
        let mut used_offsets = HashSet::new();
        for (number, getter) in numbers.into_iter().zip(getters) {
            // Generated constants and accessor methods retain declaration order in 4.6.51.
            // Confirm oneof discriminants, storage types, and WriteTo tags independently.
            let (offset, oneof_extra_data) = if getter.objects.is_empty() {
                ensure!(
                    getter.reads.len() == 1,
                    "ambiguous getter {} for field {number}",
                    getter.method.get_name()
                );
                let offset = *getter.reads.first().unwrap();
                ensure!(
                    used_offsets.insert(offset),
                    "multiple non-oneof getters use offset 0x{offset:X}"
                );
                ensure!(
                    storage[&offset] == getter.ty,
                    "getter/storage type mismatch for field {number}"
                );
                (offset, None)
            } else {
                ensure!(
                    getter.objects.len() == 1 && getter.cases.len() == 1,
                    "ambiguous oneof getter for field {number}"
                );
                let (&case_offset, &case_number) = getter.cases.first_key_value().unwrap();
                ensure!(
                    case_number == number,
                    "oneof getter disagrees with field constant: {case_number} != {number}"
                );
                ensure!(
                    storage[&case_offset].get_isenum()?.unbox(),
                    "oneof discriminator is not an enum"
                );
                (
                    getter.objects[0],
                    Some(OneofVariantInfo {
                        oneof_enum_offset: u32::try_from(case_offset)?,
                        variant_type: getter.ty,
                        property: None,
                    }),
                )
            };
            let generics = getter.ty.get_generic_arguments();
            let wire = tags.get(&number);
            if generics.is_empty() {
                ensure!(
                    wire.is_some_and(|values| values.len() == 1),
                    "missing or ambiguous WriteTo tag for field {number}"
                );
                let wire = *wire.unwrap().first().unwrap();
                let valid = match getter.ty.il_name().as_ref() {
                    "System.Boolean" => wire == 0,
                    "System.Int32" | "System.UInt32" => matches!(wire, 0 | 5),
                    "System.Int64" | "System.UInt64" => matches!(wire, 0 | 1),
                    "System.Single" => wire == 5,
                    "System.Double" => wire == 1,
                    _ if getter.ty.get_isenum()?.unbox() => wire == 0,
                    _ => wire == 2,
                };
                ensure!(
                    valid,
                    "WriteTo wire type {wire} disagrees with {} for field {number}",
                    getter.ty.il_name()
                );
            }
            let number_type = if let Some(offsets) = zigzag.get(&number) {
                ensure!(
                    offsets.len() == 1 && offsets.contains(&offset),
                    "ZigZag writer/getter offset mismatch for field {number}"
                );
                ensure!(
                    matches!(
                        getter.ty.il_name().as_ref(),
                        "System.Int32" | "System.Int64"
                    ),
                    "ZigZag field {number} is not a signed integer"
                );
                ensure!(
                    wire.is_some_and(|values| values.contains(&0)),
                    "ZigZag field {number} has a non-varint tag"
                );
                NumberType::ZigZagVarint
            } else if wire.is_some_and(|values| values.contains(&1) || values.contains(&5)) {
                NumberType::Normal
            } else {
                NumberType::None
            };
            output.fields.push(FieldMinimalInfo {
                tag: number << 3,
                xor: 0,
                offset: u32::try_from(offset)?,
                oneof_extra_data,
                number_type,
                property: None,
            });
        }
        Ok(())
    }
}

pub(super) fn code(rva: usize) -> anyhow::Result<(&'static [u8], bool)> {
    ensure!(rva != 0, "zero RVA is not executable protobuf code");
    if let Ok(bytes) = game_assembly_function(rva) {
        return Ok((bytes, false));
    }
    // Leaf accessors have no unwind record. Bound their read and stop at RET/INT3.
    let image = game_assembly_slice();
    Ok((
        image
            .get(rva..rva.saturating_add(256).min(image.len()))
            .context("getter outside GameAssembly")?,
        true,
    ))
}

fn writes(instruction: &Instruction, info: &mut InstructionInfoFactory) -> Vec<Register> {
    info.info(instruction)
        .used_registers()
        .iter()
        .filter(|used| {
            matches!(
                used.access(),
                OpAccess::Write
                    | OpAccess::CondWrite
                    | OpAccess::ReadWrite
                    | OpAccess::ReadCondWrite
            )
        })
        .map(|used| used.register().full_register())
        .collect()
}

const VOLATILE: [Register; 7] = [
    Register::RAX,
    Register::RCX,
    Register::RDX,
    Register::R8,
    Register::R9,
    Register::R10,
    Register::R11,
];

fn read_getter(
    method: Il2CppMethod,
    ty: RuntimeType,
    fields: &HashMap<usize, RuntimeType>,
) -> anyhow::Result<Option<Getter>> {
    let Ok(rva) = method.checked_rva() else {
        return Ok(None);
    };
    let (bytes, _) = code(rva)?;
    let mut owners = HashSet::from([Register::RCX]);
    let mut reads = BTreeSet::new();
    let mut cases = BTreeMap::new();
    let mut info = InstructionInfoFactory::new();
    for instruction in Decoder::with_ip(
        64,
        &bytes[..bytes.len().min(1024)],
        (*il2cpp::GA_BASE + rva) as u64,
        DecoderOptions::NONE,
    ) {
        if instruction.is_invalid()
            || matches!(instruction.mnemonic(), Mnemonic::Ret | Mnemonic::Int3)
        {
            break;
        }
        let offset = instruction.memory_displacement64() as usize;
        if owners.contains(&instruction.memory_base().full_register())
            && instruction.memory_index() == Register::None
            && fields.contains_key(&offset)
        {
            reads.insert(offset);
            if instruction.mnemonic() == Mnemonic::Cmp
                && instruction.op0_kind() == OpKind::Memory
                && is_immediate(instruction.op1_kind())
            {
                cases.insert(offset, instruction.immediate(1) as u32);
            }
        }
        let copy = instruction.mnemonic() == Mnemonic::Mov
            && instruction.op0_kind() == OpKind::Register
            && instruction.op1_kind() == OpKind::Register
            && owners.contains(&instruction.op1_register().full_register());
        for reg in writes(&instruction, &mut info) {
            owners.remove(&reg);
        }
        if copy {
            owners.insert(instruction.op0_register().full_register());
        }
        if instruction.mnemonic() == Mnemonic::Call {
            for reg in VOLATILE {
                owners.remove(&reg);
            }
        }
    }
    if reads.is_empty() {
        return Ok(None);
    }
    let objects = reads
        .iter()
        .copied()
        .filter(|offset| fields[offset].il_name() == "System.Object")
        .collect();
    Ok(Some(Getter {
        method,
        ty,
        reads,
        cases,
        objects,
    }))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Value {
    Constant(u64),
    Owner,
    Field(usize),
    Doubled(usize),
    Sign(usize),
    ZigZag(usize),
}

impl FieldDecoder {
    fn wire_types(
        &self,
        rva: usize,
    ) -> anyhow::Result<(HashMap<u32, BTreeSet<u8>>, HashMap<u32, BTreeSet<usize>>)> {
        let (bytes, leaf) = code(rva)?;
        let mut values = HashMap::from([(Register::RCX, Value::Owner)]);
        let mut tags = HashMap::new();
        let mut zigzag = HashMap::<u32, BTreeSet<usize>>::new();
        let mut last_tag = None;
        let mut info = InstructionInfoFactory::new();
        for instruction in Decoder::with_ip(
            64,
            bytes,
            (*il2cpp::GA_BASE + rva) as u64,
            DecoderOptions::NONE,
        ) {
            if instruction.is_invalid() {
                break;
            }
            if leaf && matches!(instruction.mnemonic(), Mnemonic::Ret | Mnemonic::Int3) {
                break;
            }
            let source = source(&instruction, &values);
            // A generated WriteTo writes constants only to the output byte array and
            // RIP-relative once flags. Array payload stores use registers, not tags.
            if instruction.mnemonic() == Mnemonic::Mov
                && instruction.op0_kind() == OpKind::Memory
                && instruction.memory_index() != Register::None
                && instruction.memory_displacement64() == 0x20
                && (1..=4).contains(&instruction.memory_size().size())
                && let Some(Value::Constant(value)) = source
            {
                let bytes = value.to_le_bytes();
                if let Some(tag) = record_tag(&bytes[..instruction.memory_size().size()], &mut tags)
                {
                    last_tag = Some(tag);
                }
            }
            if let Some(target) = native::direct_target(&instruction) {
                if let Some(&count) = self.raw_tags.get(&target) {
                    let bytes = [Register::RDX, Register::R8, Register::R9][..count]
                        .iter()
                        .map(|reg| match values.get(reg) {
                            Some(Value::Constant(value)) => Some(*value as u8),
                            _ => None,
                        })
                        .collect::<Option<Vec<_>>>();
                    if let Some(bytes) = bytes
                        && let Some(tag) = record_tag(&bytes, &mut tags)
                    {
                        last_tag = Some(tag);
                    }
                }
                if let Some(tag) = last_tag {
                    let offset = match values.get(&Register::RDX) {
                        Some(Value::Field(offset)) if self.sint_writers.contains(&target) => {
                            Some(*offset)
                        }
                        Some(Value::ZigZag(offset)) if self.varint_writers.contains(&target) => {
                            Some(*offset)
                        }
                        _ => None,
                    };
                    if let Some(offset) = offset {
                        zigzag.entry(tag).or_default().insert(offset);
                    }
                }
            }
            let destination = instruction.op0_register().full_register();
            let current = values.get(&destination).copied();
            let next = match instruction.mnemonic() {
                Mnemonic::Mov | Mnemonic::Movzx | Mnemonic::Movsx | Mnemonic::Movsxd => source,
                Mnemonic::Lea
                    if instruction.memory_base() == instruction.memory_index()
                        && instruction.memory_index_scale() == 1
                        && instruction.memory_displacement64() == 0 =>
                {
                    match values.get(&instruction.memory_base().full_register()) {
                        Some(Value::Field(offset)) => Some(Value::Doubled(*offset)),
                        _ => None,
                    }
                }
                Mnemonic::Shl
                    if is_immediate(instruction.op1_kind()) && instruction.immediate(1) == 1 =>
                {
                    match current {
                        Some(Value::Field(offset)) => Some(Value::Doubled(offset)),
                        _ => None,
                    }
                }
                Mnemonic::Sar
                    if is_immediate(instruction.op1_kind())
                        && matches!(instruction.immediate(1), 31 | 63) =>
                {
                    match current {
                        Some(Value::Field(offset)) => Some(Value::Sign(offset)),
                        _ => None,
                    }
                }
                Mnemonic::Xor => match (current, source) {
                    (Some(Value::Sign(left)), Some(Value::Doubled(right)))
                    | (Some(Value::Doubled(left)), Some(Value::Sign(right)))
                        if left == right =>
                    {
                        Some(Value::ZigZag(left))
                    }
                    _ if instruction.op0_register() == instruction.op1_register() => {
                        Some(Value::Constant(0))
                    }
                    _ => None,
                },
                _ => None,
            };
            for reg in writes(&instruction, &mut info) {
                values.remove(&reg);
            }
            if instruction.op0_kind() == OpKind::Register
                && let Some(value) = next
            {
                values.insert(destination, value);
            }
            if instruction.mnemonic() == Mnemonic::Call {
                for reg in VOLATILE {
                    values.remove(&reg);
                }
            }
        }
        Ok((tags, zigzag))
    }
}

fn source(instruction: &Instruction, values: &HashMap<Register, Value>) -> Option<Value> {
    if instruction.op_count() < 2 {
        return None;
    }
    match instruction.op1_kind() {
        OpKind::Register => values
            .get(&instruction.op1_register().full_register())
            .copied(),
        kind if is_immediate(kind) => Some(Value::Constant(instruction.immediate(1))),
        OpKind::Memory if instruction.memory_index() == Register::None => {
            match values.get(&instruction.memory_base().full_register()) {
                Some(Value::Owner) => {
                    Some(Value::Field(instruction.memory_displacement64() as usize))
                }
                Some(Value::Field(offset)) => Some(Value::Field(*offset)),
                _ => None,
            }
        }
        _ => None,
    }
}

fn record_tag(bytes: &[u8], output: &mut HashMap<u32, BTreeSet<u8>>) -> Option<u32> {
    let mut tag = 0u32;
    for (index, &byte) in bytes.iter().take(5).enumerate() {
        tag |= u32::from(byte & 0x7f) << (7 * index);
        if byte < 0x80 {
            let number = tag >> 3;
            let wire = (tag & 7) as u8;
            if number != 0 && matches!(wire, 0 | 1 | 2 | 5) {
                output.entry(number).or_default().insert(wire);
                return Some(number);
            }
            break;
        }
    }
    None
}

fn is_immediate(kind: OpKind) -> bool {
    matches!(
        kind,
        OpKind::Immediate8
            | OpKind::Immediate16
            | OpKind::Immediate32
            | OpKind::Immediate64
            | OpKind::Immediate8to16
            | OpKind::Immediate8to32
            | OpKind::Immediate8to64
            | OpKind::Immediate32to64
    )
}
