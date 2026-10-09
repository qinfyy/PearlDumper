use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use iced_x86::{
    Decoder, DecoderOptions, FlowControl, Instruction, InstructionInfoFactory, Mnemonic, OpAccess,
    OpKind, Register,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Value {
    This,
    Stack(i64),
    Object(u64),
    Field(u64, u32),
    Chars(u64, u32),
    Length(u64, u32),
    Decoded(u64, u32, u64),
    Key(u64, u32, u64, u64),
    Null,
}

#[derive(Clone, Default, Eq, PartialEq)]
struct State {
    regs: HashMap<Register, Value>,
    stack: BTreeMap<i64, Value>,
}

impl State {
    fn get(&self, r: Register) -> Option<Value> {
        self.regs.get(&r.full_register()).copied().filter(|v| {
            r.is_gpr64() || (r.is_gpr32() && matches!(v, Value::Length(..) | Value::Null))
        })
    }
    fn merge(&mut self, other: &Self) -> bool {
        let before = self.clone();
        // Empty input produces null, not a second key source. All non-null
        // paths must still agree on the field and both conversion call sites.
        self.regs
            .retain(|reg, value| match (*value, other.regs.get(reg).copied()) {
                (Value::Null, Some(key @ Value::Key(..))) => {
                    *value = key;
                    true
                }
                (Value::Key(..), Some(Value::Null)) => true,
                (_, Some(v)) => *value == v,
                _ => false,
            });
        self.stack.retain(|k, v| other.stack.get(k) == Some(v));
        *self != before
    }
}

pub(super) struct Layout {
    pub parsers: Vec<(usize, Register)>,
    pub base64_chars: usize,
    pub base64_string: usize,
    pub string_length: u32,
    pub string_chars: u32,
    pub key_offset: u32,
}

#[derive(Debug)]
pub(super) struct Observation {
    pub offset: u32,
    pub decode: usize,
    pub transform: usize,
    pub store: usize,
}

pub(super) fn recover(
    bytes: &[u8],
    ip: usize,
    layout: &Layout,
) -> anyhow::Result<Vec<Observation>> {
    let instructions: Vec<_> = Decoder::with_ip(64, bytes, ip as u64, DecoderOptions::NONE)
        .into_iter()
        .collect();
    anyhow::ensure!(
        !instructions.is_empty() && instructions.iter().all(|i| !i.is_invalid()),
        "invalid gateway consumer body"
    );
    let indices: HashMap<_, _> = instructions
        .iter()
        .enumerate()
        .map(|(n, i)| (i.ip(), n))
        .collect();
    let mut successors = vec![Vec::new(); instructions.len()];
    for (n, i) in instructions.iter().enumerate() {
        anyhow::ensure!(
            i.flow_control() != FlowControl::IndirectBranch,
            "unresolved gateway consumer branch"
        );
        if matches!(
            i.flow_control(),
            FlowControl::Next
                | FlowControl::Call
                | FlowControl::IndirectCall
                | FlowControl::ConditionalBranch
        ) && n + 1 < instructions.len()
        {
            successors[n].push(n + 1);
        }
        if matches!(
            i.flow_control(),
            FlowControl::ConditionalBranch | FlowControl::UnconditionalBranch
        ) {
            if let Some(&next) = indices.get(&i.near_branch_target()) {
                successors[n].push(next);
            } else {
                anyhow::ensure!(
                    i.flow_control() == FlowControl::UnconditionalBranch,
                    "gateway consumer conditional branch leaves function"
                );
            }
        }
    }
    let mut incoming = vec![None; instructions.len()];
    incoming[0] = Some(State {
        regs: HashMap::from([
            (Register::RCX, Value::This),
            (Register::RSP, Value::Stack(0)),
        ]),
        stack: BTreeMap::new(),
    });
    let mut queue = VecDeque::from([0]);
    let mut factory = InstructionInfoFactory::new();
    let mut steps = 0;
    while let Some(n) = queue.pop_front() {
        steps += 1;
        anyhow::ensure!(
            steps <= instructions.len() * 128,
            "gateway consumer did not converge"
        );
        let mut state = incoming[n].clone().unwrap();
        transfer(&instructions[n], &mut state, layout, &mut factory);
        for &next in &successors[n] {
            let changed = if let Some(previous) = &mut incoming[next] {
                previous.merge(&state)
            } else {
                incoming[next] = Some(state.clone());
                true
            };
            if changed {
                queue.push_back(next);
            }
        }
    }
    let mut roots = BTreeSet::new();
    for (i, state) in instructions.iter().zip(&incoming) {
        let Some(state) = state else { continue };
        for &(target, argument) in &layout.parsers {
            if crate::proto::native::direct_target(i) == Some(target)
                && let Some(Value::Object(root)) = state.get(argument)
            {
                roots.insert(root);
            }
        }
    }
    let mut result = Vec::new();
    for (i, state) in instructions.iter().zip(incoming) {
        let Some(state) = state else { continue };
        if i.mnemonic() == Mnemonic::Mov
            && i.op0_kind() == OpKind::Memory
            && i.op1_kind() == OpKind::Register
            && i.memory_index() == Register::None
            && i.memory_displacement32() == layout.key_offset
            && i.memory_size().size() == 8
            && state.get(i.memory_base()) == Some(Value::This)
            && let Some(Value::Key(root, offset, decode, transform)) = state.get(i.op1_register())
            && roots.contains(&root)
        {
            result.push(Observation {
                offset,
                decode: decode as usize,
                transform: transform as usize,
                store: i.ip() as usize,
            });
        }
    }
    Ok(result)
}

fn transfer(
    i: &Instruction,
    state: &mut State,
    layout: &Layout,
    factory: &mut InstructionInfoFactory,
) {
    let before = state.clone();
    let base = before.get(i.memory_base());
    let displacement = i.memory_displacement32();
    let value = if i.mnemonic() == Mnemonic::Mov {
        match i.op1_kind() {
            OpKind::Register => before.get(i.op1_register()),
            OpKind::Memory if i.memory_index() == Register::None => match base {
                Some(Value::Object(root)) if i.memory_size().size() == 8 => {
                    Some(Value::Field(root, displacement))
                }
                Some(Value::Field(root, offset))
                    if displacement == layout.string_length && i.memory_size().size() == 4 =>
                {
                    Some(Value::Length(root, offset))
                }
                Some(Value::Stack(sp)) if i.memory_size().size() == 8 => before
                    .stack
                    .get(&(sp + i.memory_displacement64() as i64))
                    .copied(),
                _ => None,
            },
            _ => None,
        }
    } else {
        None
    };
    for used in factory.info(i).used_registers() {
        if matches!(
            used.access(),
            OpAccess::Write | OpAccess::CondWrite | OpAccess::ReadWrite | OpAccess::ReadCondWrite
        ) {
            state.regs.remove(&used.register().full_register());
        }
    }
    if i.op0_kind() == OpKind::Register
        && let Some(value) = value
        && (i.op0_register().is_gpr64()
            || (i.op0_register().is_gpr32() && matches!(value, Value::Length(..) | Value::Null)))
    {
        state.regs.insert(i.op0_register().full_register(), value);
    }
    if i.mnemonic() == Mnemonic::Xor
        && i.op0_kind() == OpKind::Register
        && i.op1_kind() == OpKind::Register
        && i.op0_register() == i.op1_register()
        && (i.op0_register().is_gpr32() || i.op0_register().is_gpr64())
    {
        state
            .regs
            .insert(i.op0_register().full_register(), Value::Null);
    }
    if matches!(i.mnemonic(), Mnemonic::Add | Mnemonic::Sub)
        && i.op0_kind() == OpKind::Register
        && i.op0_register().is_gpr64()
        && matches!(
            i.op1_kind(),
            OpKind::Immediate8to64 | OpKind::Immediate32to64
        )
    {
        let delta = i.immediate(1) as i64 * if i.mnemonic() == Mnemonic::Sub { -1 } else { 1 };
        let result = match before.get(i.op0_register()) {
            Some(Value::Stack(sp)) => Some(Value::Stack(sp + delta)),
            Some(Value::Field(root, offset)) if delta == layout.string_chars as i64 => {
                Some(Value::Chars(root, offset))
            }
            _ => None,
        };
        if let Some(v) = result {
            state.regs.insert(i.op0_register(), v);
        }
    }
    if i.mnemonic() == Mnemonic::Lea
        && i.memory_index() == Register::None
        && i.op0_register().is_gpr64()
    {
        let result = match base {
            Some(Value::Stack(sp)) => Some(Value::Stack(sp + i.memory_displacement64() as i64)),
            Some(Value::Field(root, offset)) if displacement == layout.string_chars => {
                Some(Value::Chars(root, offset))
            }
            _ => None,
        };
        if let Some(v) = result {
            state.regs.insert(i.op0_register(), v);
        }
    }
    if matches!(i.mnemonic(), Mnemonic::Push | Mnemonic::Pop) {
        if let Some(Value::Stack(sp)) = before.get(Register::RSP) {
            state.regs.insert(
                Register::RSP,
                Value::Stack(
                    sp + if i.mnemonic() == Mnemonic::Push {
                        -8
                    } else {
                        8
                    },
                ),
            );
        }
        state.stack.clear();
    }
    if i.op0_kind() == OpKind::Memory {
        if let Some(Value::Stack(sp)) = base {
            if i.memory_index() == Register::None {
                let start = sp + i.memory_displacement64() as i64;
                state
                    .stack
                    .retain(|k, _| *k + 8 <= start || *k >= start + i.memory_size().size() as i64);
                if i.mnemonic() == Mnemonic::Mov
                    && i.memory_size().size() == 8
                    && let Some(v) = value
                {
                    state.stack.insert(start, v);
                }
            } else {
                state.stack.clear();
            }
        } else if !matches!(base, Some(Value::This | Value::Object(_))) {
            state.stack.clear();
        }
    }
    if matches!(
        i.flow_control(),
        FlowControl::Call | FlowControl::IndirectCall
    ) {
        let target = crate::proto::native::direct_target(i);
        let rcx = before.get(Register::RCX);
        let result = match rcx {
            Some(Value::Chars(root, offset))
                if target == Some(layout.base64_chars)
                    && before.get(Register::EDX) == Some(Value::Length(root, offset)) =>
            {
                Value::Decoded(root, offset, i.ip())
            }
            Some(Value::Field(root, offset)) if target == Some(layout.base64_string) => {
                Value::Decoded(root, offset, i.ip())
            }
            Some(Value::Decoded(root, offset, decode))
                if i.flow_control() == FlowControl::IndirectCall
                    && i.is_ip_rel_memory_operand() =>
            {
                Value::Key(root, offset, decode, i.ip())
            }
            _ => Value::Object(i.ip()),
        };
        for r in [
            Register::RAX,
            Register::RCX,
            Register::RDX,
            Register::R8,
            Register::R9,
            Register::R10,
            Register::R11,
        ] {
            state.regs.remove(&r);
        }
        state.regs.insert(Register::RAX, result);
        if let Some(sp) = before.get(Register::RSP) {
            state.regs.insert(Register::RSP, sp);
        }
        state.stack.clear();
    }
}
