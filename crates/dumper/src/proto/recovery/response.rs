use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

use iced_x86::{
    Decoder, DecoderOptions, FlowControl, Instruction, InstructionInfoFactory, Mnemonic, OpAccess,
    OpKind, Register,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Value {
    Argument,
    Header,
    Slot(usize),
    Stack(i64),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Check {
    Null,
    Class(usize),
}

#[derive(Clone, Default, Eq, PartialEq)]
struct State {
    regs: HashMap<Register, Value>,
    stack: HashMap<i64, Value>,
    check: Option<Check>,
    failure: Option<Check>,
}

impl State {
    fn get(&self, reg: Register) -> Option<Value> {
        reg.is_gpr64()
            .then(|| self.regs.get(&reg).copied())
            .flatten()
    }

    fn merge(&mut self, other: &Self) -> bool {
        let before = self.clone();
        self.regs.retain(|r, v| other.regs.get(r) == Some(v));
        self.stack.retain(|r, v| other.stack.get(r) == Some(v));
        if self.check != other.check {
            self.check = None;
        }
        // A null argument and a failed cast share the same diagnostic block.
        self.failure = match (self.failure, other.failure) {
            (Some(Check::Null), class @ Some(Check::Class(_)))
            | (class @ Some(Check::Class(_)), Some(Check::Null)) => class,
            (a, b) if a == b => a,
            _ => None,
        };
        *self != before
    }
}

#[derive(Debug)]
pub(crate) struct Diagnostic {
    pub class: usize,
    pub literal: String,
    pub slot: usize,
    pub call: usize,
}

#[derive(Default, Debug)]
pub(crate) struct Analysis {
    pub classes: BTreeSet<usize>,
    pub diagnostics: Vec<Diagnostic>,
}

fn operand(i: &Instruction, op: u32, state: &State) -> Option<Value> {
    match i.op_kind(op) {
        OpKind::Register => state.get(i.op_register(op)),
        OpKind::Memory if i.memory_size().size() == 8 => {
            if i.is_ip_rel_memory_operand() {
                return Some(Value::Slot(i.ip_rel_memory_address() as usize));
            }
            if i.memory_index() != Register::None {
                return None;
            }
            match state.get(i.memory_base()) {
                Some(Value::Argument) if i.memory_displacement64() == 0 => Some(Value::Header),
                Some(Value::Stack(sp)) => state
                    .stack
                    .get(&(sp + i.memory_displacement64() as i64))
                    .copied(),
                _ => None,
            }
        }
        _ => None,
    }
}

fn check(i: &Instruction, state: &State, class: &impl Fn(usize) -> Option<usize>) -> Option<Check> {
    let a = operand(i, 0, state);
    let b = operand(i, 1, state);
    match (i.mnemonic(), a, b) {
        (Mnemonic::Cmp, Some(Value::Header), Some(Value::Slot(slot)))
        | (Mnemonic::Cmp, Some(Value::Slot(slot)), Some(Value::Header)) => {
            class(slot).map(Check::Class)
        }
        (Mnemonic::Test, Some(Value::Argument), Some(Value::Argument)) => Some(Check::Null),
        _ => None,
    }
}

pub(crate) fn analyze(
    bytes: &[u8],
    ip: usize,
    argument: Register,
    class: impl Fn(usize) -> Option<usize>,
    literal: impl Fn(usize) -> Option<String>,
    loggers: &HashMap<usize, Register>,
    no_return: &HashSet<usize>,
) -> anyhow::Result<Analysis> {
    let code: Vec<_> = Decoder::with_ip(64, bytes, ip as u64, DecoderOptions::NONE)
        .into_iter()
        .collect();
    anyhow::ensure!(
        !code.is_empty() && code.iter().all(|i| !i.is_invalid()),
        "invalid response handler"
    );
    let indices: HashMap<_, _> = code.iter().enumerate().map(|(n, i)| (i.ip(), n)).collect();
    let mut edges = vec![Vec::new(); code.len()];
    for (n, i) in code.iter().enumerate() {
        let terminates = i.flow_control() == FlowControl::Call
            && crate::proto::native::direct_target(i).is_some_and(|t| no_return.contains(&t));
        if !terminates
            && matches!(
                i.flow_control(),
                FlowControl::Next
                    | FlowControl::Call
                    | FlowControl::IndirectCall
                    | FlowControl::ConditionalBranch
            )
            && n + 1 < code.len()
        {
            edges[n].push((n + 1, false));
        }
        if matches!(
            i.flow_control(),
            FlowControl::ConditionalBranch | FlowControl::UnconditionalBranch
        ) {
            if let Some(&target) = indices.get(&i.near_branch_target()) {
                edges[n].push((target, true));
            } else {
                anyhow::ensure!(
                    i.flow_control() == FlowControl::UnconditionalBranch,
                    "response conditional branch leaves function"
                );
            }
        }
    }
    let mut incoming = vec![None; code.len()];
    incoming[0] = Some(State {
        regs: HashMap::from([
            (argument, Value::Argument),
            (Register::RSP, Value::Stack(0)),
        ]),
        ..Default::default()
    });
    let mut pending = VecDeque::from([0]);
    let mut info = InstructionInfoFactory::new();
    let mut steps = 0;
    while let Some(n) = pending.pop_front() {
        steps += 1;
        anyhow::ensure!(
            steps <= code.len() * 128,
            "response data flow did not converge"
        );
        let i = &code[n];
        let before = incoming[n].clone().unwrap();
        let mut after = before.clone();
        transfer(i, &mut after, &class, &mut info);
        for &(next, taken) in &edges[n] {
            let mut state = after.clone();
            if matches!(i.mnemonic(), Mnemonic::Je | Mnemonic::Jne) {
                let equal = taken == (i.mnemonic() == Mnemonic::Je);
                if let Some(check) = before.check {
                    state.failure = match check {
                        Check::Null if equal => Some(Check::Null),
                        Check::Class(_) if !equal => Some(check),
                        _ => None,
                    };
                }
            }
            let changed = if let Some(old) = &mut incoming[next] {
                old.merge(&state)
            } else {
                incoming[next] = Some(state);
                true
            };
            if changed {
                pending.push_back(next);
            }
        }
    }
    let mut result = Analysis::default();
    for (n, (i, state)) in code.iter().zip(&incoming).enumerate() {
        let Some(state) = state else { continue };
        if i.flow_control() == FlowControl::IndirectBranch {
            anyhow::ensure!(
                state.get(Register::RSP) == Some(Value::Stack(0)) && tail_epilogue(&code, n),
                "unresolved response branch at 0x{:X}",
                i.ip()
            );
        }
        if let Some(Check::Class(class)) = check(i, state, &class) {
            result.classes.insert(class);
        }
        if let Some(target) = crate::proto::native::direct_target(i)
            && let Some(&reg) = loggers.get(&target)
            && let Some(Value::Slot(slot)) = state.get(reg)
            && let Some(Check::Class(class)) = state.failure
            && let Some(literal) = literal(slot)
        {
            result.diagnostics.push(Diagnostic {
                class,
                literal,
                slot,
                call: i.ip() as usize,
            });
        }
    }
    Ok(result)
}

fn tail_epilogue(code: &[Instruction], n: usize) -> bool {
    if code[n].op0_kind() != OpKind::Register || !code[n].op0_register().is_gpr64() {
        return false;
    }
    let mut cursor = n;
    while cursor > 0 && code[cursor - 1].mnemonic() == Mnemonic::Pop {
        if !matches!(
            code[cursor - 1].op0_register(),
            Register::RBX
                | Register::RBP
                | Register::RSI
                | Register::RDI
                | Register::R12
                | Register::R13
                | Register::R14
                | Register::R15
        ) {
            return false;
        }
        cursor -= 1;
    }
    let Some(i) = cursor.checked_sub(1).map(|p| &code[p]) else {
        return false;
    };
    i.mnemonic() == Mnemonic::Add
        && i.op0_register() == Register::RSP
        && matches!(
            i.op1_kind(),
            OpKind::Immediate8to64 | OpKind::Immediate32to64
        )
        && (i.immediate(1) as i64) > 0
}

// Only complete small helper bodies qualify. A call that returns still reaches
// a trap; no return instruction, external jump, or fallthrough exit is allowed.
pub(crate) fn trap_terminated(bytes: &[u8], ip: usize) -> bool {
    if bytes.is_empty() || bytes.len() > 4096 {
        return false;
    }
    let code: Vec<_> = Decoder::with_ip(64, bytes, ip as u64, DecoderOptions::NONE)
        .into_iter()
        .collect();
    if !code
        .last()
        .is_some_and(|i| matches!(i.mnemonic(), Mnemonic::Int3 | Mnemonic::Ud2))
    {
        return false;
    }
    let starts: HashSet<_> = code.iter().map(|i| i.ip()).collect();
    code.iter().all(|i| {
        !i.is_invalid()
            && match i.flow_control() {
                FlowControl::Next | FlowControl::Call | FlowControl::IndirectCall => true,
                FlowControl::ConditionalBranch | FlowControl::UnconditionalBranch => {
                    starts.contains(&i.near_branch_target())
                }
                _ => matches!(i.mnemonic(), Mnemonic::Int3 | Mnemonic::Ud2),
            }
    })
}

fn transfer(
    i: &Instruction,
    state: &mut State,
    class: &impl Fn(usize) -> Option<usize>,
    info: &mut InstructionInfoFactory,
) {
    let before = state.clone();
    let value = (i.mnemonic() == Mnemonic::Mov)
        .then(|| operand(i, 1, &before))
        .flatten();
    for r in info.info(i).used_registers() {
        if matches!(
            r.access(),
            OpAccess::Write | OpAccess::CondWrite | OpAccess::ReadWrite | OpAccess::ReadCondWrite
        ) {
            state.regs.remove(&r.register().full_register());
        }
    }
    if i.rflags_modified() != 0 {
        state.check = check(i, &before, class);
    }
    if i.op0_kind() == OpKind::Register
        && i.op0_register().is_gpr64()
        && let Some(v) = value
    {
        state.regs.insert(i.op0_register(), v);
    }
    if i.mnemonic() == Mnemonic::Lea
        && i.memory_index() == Register::None
        && i.op0_register().is_gpr64()
        && let Some(Value::Stack(sp)) = before.get(i.memory_base())
    {
        state.regs.insert(
            i.op0_register(),
            Value::Stack(sp + i.memory_displacement64() as i64),
        );
    }
    if matches!(i.mnemonic(), Mnemonic::Add | Mnemonic::Sub)
        && i.op0_kind() == OpKind::Register
        && i.op0_register().is_gpr64()
        && matches!(
            i.op1_kind(),
            OpKind::Immediate8to64 | OpKind::Immediate32to64
        )
        && let Some(Value::Stack(sp)) = before.get(i.op0_register())
    {
        let delta = i.immediate(1) as i64 * if i.mnemonic() == Mnemonic::Sub { -1 } else { 1 };
        state
            .regs
            .insert(i.op0_register(), Value::Stack(sp + delta));
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
    let writes_memory = info.info(i).used_memory().iter().any(|m| {
        matches!(
            m.access(),
            OpAccess::Write | OpAccess::CondWrite | OpAccess::ReadWrite | OpAccess::ReadCondWrite
        )
    });
    if writes_memory && i.op0_kind() == OpKind::Memory {
        if i.memory_index() == Register::None
            && let Some(Value::Stack(sp)) = before.get(i.memory_base())
        {
            let start = sp + i.memory_displacement64() as i64;
            state
                .stack
                .retain(|p, _| *p + 8 <= start || *p >= start + i.memory_size().size() as i64);
            if i.memory_size().size() == 8
                && let Some(v) = value
            {
                state.stack.insert(start, v);
            }
        } else {
            state.stack.clear();
        }
    }
    if matches!(
        i.flow_control(),
        FlowControl::Call | FlowControl::IndirectCall
    ) {
        for reg in [
            Register::RAX,
            Register::RCX,
            Register::RDX,
            Register::R8,
            Register::R9,
            Register::R10,
            Register::R11,
        ] {
            state.regs.remove(&reg);
        }
        if let Some(sp) = before.get(Register::RSP) {
            state.regs.insert(Register::RSP, sp);
        }
        state.check = None;
        state.stack.clear();
    }
}
