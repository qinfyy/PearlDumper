use std::collections::{BTreeMap, HashMap, HashSet};

use iced_x86::{
    Decoder, DecoderOptions, Instruction, InstructionInfoFactory, Mnemonic, OpAccess, OpKind,
    Register,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct SendRegisters {
    pub id: Register,
    pub message: Register,
}

impl SendRegisters {
    pub const STANDARD: Self = Self {
        id: Register::RDX,
        message: Register::R8,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct PacketRegistration {
    pub class_slot: usize,
    pub type_offset: usize,
    pub id: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct HandlerRegistration {
    pub method_slot: usize,
    pub id: u16,
}

#[derive(Default)]
pub(super) struct Registrations {
    pub fields: BTreeMap<usize, usize>,
    pub packets: BTreeMap<usize, Vec<PacketRegistration>>,
    pub handlers: BTreeMap<usize, Vec<HandlerRegistration>>,
    pub requests: Vec<PacketRegistration>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Value {
    Constant(u64),
    Slot { address: usize, offset: usize },
    Object(usize),
}

#[derive(Default)]
struct Object {
    class_slot: Option<usize>,
    method_slot: Option<usize>,
}

#[derive(Default)]
struct State {
    registers: HashMap<Register, Value>,
    stack: HashMap<i64, Value>,
    stack_pointer: i64,
    objects: Vec<Object>,
}

impl State {
    fn get(&self, register: Register) -> Option<Value> {
        self.registers.get(&register.full_register()).copied()
    }

    fn set(&mut self, register: Register, value: Option<Value>) {
        let register = register.full_register();
        self.registers.remove(&register);
        if register.is_gpr64()
            && let Some(value) = value
        {
            self.registers.insert(register, value);
        }
    }

    fn source(&self, instruction: &Instruction) -> Option<Value> {
        match instruction.op1_kind() {
            OpKind::Register => self.get(instruction.op1_register()),
            OpKind::Immediate8
            | OpKind::Immediate16
            | OpKind::Immediate32
            | OpKind::Immediate64
            | OpKind::Immediate8to16
            | OpKind::Immediate8to32
            | OpKind::Immediate8to64
            | OpKind::Immediate32to64 => Some(Value::Constant(instruction.immediate(1))),
            OpKind::Memory
                if instruction.is_ip_rel_memory_operand()
                    && instruction.memory_size().size() == 8 =>
            {
                Some(Value::Slot {
                    address: instruction.ip_rel_memory_address() as usize,
                    offset: 0,
                })
            }
            OpKind::Memory
                if instruction.memory_base() == Register::RSP
                    && instruction.memory_index() == Register::None =>
            {
                self.stack
                    .get(&(self.stack_pointer + instruction.memory_displacement64() as i64))
                    .copied()
            }
            _ => None,
        }
    }

    fn clear_volatile(&mut self) {
        for reg in [
            Register::RAX,
            Register::RCX,
            Register::RDX,
            Register::R8,
            Register::R9,
            Register::R10,
            Register::R11,
        ] {
            self.registers.remove(&reg);
        }
    }

    fn packet(&self, id: Register, message: Register) -> Option<PacketRegistration> {
        let Value::Constant(id) = self.get(id)? else {
            return None;
        };
        let id = u16::try_from(id).ok().filter(|id| *id != 0)?;
        let Value::Object(object) = self.get(message)? else {
            return None;
        };
        Some(PacketRegistration {
            class_slot: self.objects.get(object)?.class_slot?,
            type_offset: 0,
            id,
        })
    }
}

pub(super) fn direct_target(instruction: &Instruction) -> Option<usize> {
    (matches!(instruction.mnemonic(), Mnemonic::Call | Mnemonic::Jmp)
        && matches!(
            instruction.op0_kind(),
            OpKind::NearBranch16 | OpKind::NearBranch32 | OpKind::NearBranch64
        ))
    .then(|| instruction.near_branch_target() as usize)
}

pub(super) fn metadata_usage_lists(bytes: &[u8], ip: usize, helper: usize) -> Vec<u32> {
    let mut result = Vec::new();
    let mut decoder = Decoder::with_ip(64, bytes, ip as u64, DecoderOptions::NONE);
    let mut previous = [Instruction::default(); 2];
    while decoder.can_decode() {
        let instruction = decoder.decode();
        // The cold block is MOV ECX, usage-list; CALL InitializeMethodMetadata;
        // MOV byte ptr [RIP+guard], 1. Do not execute the surrounding game method.
        if previous[0].mnemonic() == Mnemonic::Mov
            && previous[0].op0_register() == Register::ECX
            && previous[0].op1_kind() == OpKind::Immediate32
            && direct_target(&previous[1]) == Some(helper)
            && previous[1].mnemonic() == Mnemonic::Call
            && instruction.mnemonic() == Mnemonic::Mov
            && instruction.op0_kind() == OpKind::Memory
            && instruction.is_ip_rel_memory_operand()
            && instruction.memory_size().size() == 1
            && instruction.op1_kind() == OpKind::Immediate8
            && instruction.immediate8() == 1
        {
            result.push(previous[0].immediate32());
        }
        previous = [previous[1], instruction];
    }
    result.sort_unstable();
    result.dedup();
    result
}

// This tracks the compiler's registry construction idioms, not arbitrary program execution.
// Packet IDs are checked against the complete type registry before request names are accepted.
pub(super) fn scan(
    bytes: &[u8],
    ip: usize,
    allocation: usize,
    field_offsets: &HashSet<usize>,
    sends: &HashMap<usize, SendRegisters>,
) -> anyhow::Result<Registrations> {
    let mut state = State::default();
    let mut output = Registrations::default();
    let mut decoder = Decoder::with_ip(64, bytes, ip as u64, DecoderOptions::NONE);
    let mut info = InstructionInfoFactory::new();
    while decoder.can_decode() {
        let instruction = decoder.decode();
        anyhow::ensure!(
            !instruction.is_invalid(),
            "invalid instruction at 0x{:X}",
            instruction.ip()
        );
        match instruction.mnemonic() {
            Mnemonic::Mov | Mnemonic::Movzx | Mnemonic::Movsx | Mnemonic::Movsxd => {
                let value = state.source(&instruction);
                if instruction.op0_kind() == OpKind::Register {
                    state.set(instruction.op0_register(), value);
                } else if instruction.op0_kind() == OpKind::Memory
                    && instruction.memory_index() == Register::None
                {
                    let offset = instruction.memory_displacement64() as usize;
                    if instruction.memory_base() == Register::RSP {
                        let key = state.stack_pointer + offset as i64;
                        state.stack.remove(&key);
                        if let Some(value) = value {
                            state.stack.insert(key, value);
                        }
                    } else if instruction.memory_size().size() == 8 {
                        if offset == 0x28
                            && let Some(Value::Object(object)) =
                                state.get(instruction.memory_base())
                            && let Some(Value::Slot { address, offset: 0 }) = value
                        {
                            // System.Delegate.method_info is stored at +0x28 in this runtime.
                            state.objects[object].method_slot = Some(address);
                        }
                        if field_offsets.contains(&offset)
                            && let Some(Value::Object(object)) = value
                            && let Some(slot) = state.objects[object].method_slot
                        {
                            let previous = output.fields.insert(offset, slot);
                            anyhow::ensure!(
                                previous.is_none_or(|old| old == slot),
                                "conflicting XLua delegate at static offset 0x{offset:X}"
                            );
                        }
                    }
                }
            }
            Mnemonic::Add | Mnemonic::Sub
                if instruction.op0_kind() == OpKind::Register
                    && is_immediate(instruction.op1_kind()) =>
            {
                let register = instruction.op0_register();
                let delta = instruction.immediate(1) as i64
                    * if instruction.mnemonic() == Mnemonic::Sub {
                        -1
                    } else {
                        1
                    };
                if register == Register::RSP {
                    state.stack_pointer += delta;
                } else {
                    let value = match state.get(register) {
                        Some(Value::Slot { address, offset }) => offset
                            .checked_add_signed(delta as isize)
                            .map(|offset| Value::Slot { address, offset }),
                        Some(Value::Constant(value)) => {
                            value.checked_add_signed(delta).map(Value::Constant)
                        }
                        _ => None,
                    };
                    state.set(register, value);
                }
            }
            Mnemonic::Xor
                if instruction.op0_kind() == OpKind::Register
                    && instruction.op0_register() == instruction.op1_register() =>
            {
                state.set(instruction.op0_register(), Some(Value::Constant(0)));
            }
            Mnemonic::Push => state.stack_pointer -= 8,
            Mnemonic::Pop => {
                state.stack_pointer += 8;
                state.set(instruction.op0_register(), None);
            }
            Mnemonic::Call | Mnemonic::Jmp => {
                if let Some(target) = direct_target(&instruction) {
                    if let Some(abi) = sends.get(&target)
                        && let Some(packet) = state.packet(abi.id, abi.message)
                    {
                        output.requests.push(packet);
                    }
                    if let (
                        Some(Value::Slot {
                            address,
                            offset: 0x30,
                        }),
                        Some(Value::Constant(id)),
                    ) = (state.get(Register::RDX), state.get(Register::R8))
                        && let Ok(id @ 1..) = u16::try_from(id)
                    {
                        output
                            .packets
                            .entry(target)
                            .or_default()
                            .push(PacketRegistration {
                                class_slot: address,
                                type_offset: 0x30,
                                id,
                            });
                    }
                    if let (Some(Value::Constant(id)), Some(Value::Object(object))) =
                        (state.get(Register::RDX), state.get(Register::R8))
                        && let Ok(id @ 1..) = u16::try_from(id)
                        && let Some(method_slot) = state.objects[object].method_slot
                    {
                        output
                            .handlers
                            .entry(target)
                            .or_default()
                            .push(HandlerRegistration { method_slot, id });
                    }
                }
                if instruction.mnemonic() == Mnemonic::Call {
                    let class_slot = match state.get(Register::RCX) {
                        Some(Value::Slot { address, offset: 0 }) => Some(address),
                        _ => None,
                    };
                    state.clear_volatile();
                    if direct_target(&instruction) == Some(allocation) {
                        let object = state.objects.len();
                        state.objects.push(Object {
                            class_slot,
                            method_slot: None,
                        });
                        state.set(Register::RAX, Some(Value::Object(object)));
                    }
                }
            }
            _ => {
                // Unknown writes must invalidate provenance; keeping an old RAX after a call
                // or arithmetic instruction can associate a command with the wrong message.
                for used in info.info(&instruction).used_registers() {
                    if matches!(
                        used.access(),
                        OpAccess::Write
                            | OpAccess::CondWrite
                            | OpAccess::ReadWrite
                            | OpAccess::ReadCondWrite
                    ) {
                        state.set(used.register(), None);
                    }
                }
            }
        }
    }
    Ok(output)
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

pub(super) fn reference_functions(
    regions: impl IntoIterator<Item = (usize, &'static [u8])>,
    base: usize,
    targets: &HashMap<usize, SendRegisters>,
    functions: &utils::pe::FunctionTable,
) -> std::collections::BTreeSet<usize> {
    let mut result = std::collections::BTreeSet::new();
    for (rva, bytes) in regions {
        for (offset, byte) in bytes.iter().enumerate() {
            if !matches!(byte, 0xe8 | 0xe9) {
                continue;
            }
            let Some(displacement) = bytes
                .get(offset + 1..offset + 5)
                .and_then(|bytes| <[u8; 4]>::try_from(bytes).ok())
            else {
                continue;
            };
            let Some(target) = (base + rva + offset + 5)
                .checked_add_signed(i32::from_le_bytes(displacement) as isize)
            else {
                continue;
            };
            if targets.contains_key(&target)
                && let Some(function) = functions.containing(rva + offset)
            {
                result.insert(function.start as usize);
            }
        }
    }
    result
}
