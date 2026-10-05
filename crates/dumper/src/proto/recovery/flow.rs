use iced_x86::{
    Decoder, DecoderOptions, FlowControl, Instruction, InstructionInfoFactory, Mnemonic, OpAccess,
    OpKind, Register,
};
use std::collections::{BTreeMap, HashMap, VecDeque};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Origin {
    Proto,
    Game,
    Class(usize),
    Field(u32, usize),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct State {
    regs: HashMap<Register, Origin>,
    stack: BTreeMap<i64, Origin>,
    sp: Option<i64>,
}

impl State {
    fn get(&self, r: Register) -> Option<Origin> {
        self.regs
            .get(&r.full_register())
            .copied()
            .filter(|v| match v {
                Origin::Field(_, width) => {
                    r.size() >= *width
                        && !matches!(r, Register::AH | Register::BH | Register::CH | Register::DH)
                }
                _ => r.is_gpr64(),
            })
    }
    fn merge(&mut self, other: &Self) -> bool {
        let before = self.clone();
        self.regs.retain(|r, v| other.regs.get(r) == Some(v));
        if self.sp != other.sp {
            self.sp = None;
            self.stack.clear();
        } else {
            self.stack.retain(|k, v| other.stack.get(k) == Some(v));
        }
        *self != before
    }
}

pub(crate) struct FieldCopy {
    pub proto_offset: u32,
    pub game_offset: u32,
    pub instruction: usize,
}

pub(crate) struct ParserLayout {
    pub proto_argument: Register,
    pub game_argument: Option<Register>,
    pub allocation: usize,
    pub game_class: usize,
}

// Forward must-analysis: only facts identical on every incoming path survive a join.
// Emit observations after convergence so an early traversal cannot leak stale facts.
pub(crate) fn field_copies(
    bytes: &[u8],
    ip: usize,
    layout: ParserLayout,
    class_from_slot: impl Fn(usize) -> Option<usize>,
) -> anyhow::Result<Vec<FieldCopy>> {
    let instructions: Vec<_> = Decoder::with_ip(64, bytes, ip as u64, DecoderOptions::NONE)
        .into_iter()
        .collect();
    anyhow::ensure!(
        instructions.iter().all(|i| !i.is_invalid()),
        "invalid instruction in NT parser at 0x{ip:X}"
    );
    if instructions.is_empty() {
        return Ok(Vec::new());
    }
    let indices: HashMap<_, _> = instructions
        .iter()
        .enumerate()
        .map(|(n, i)| (i.ip(), n))
        .collect();
    let mut successors = vec![Vec::new(); instructions.len()];
    for (n, i) in instructions.iter().enumerate() {
        anyhow::ensure!(
            i.flow_control() != FlowControl::IndirectBranch,
            "unresolved indirect branch in NT parser at 0x{:X}",
            i.ip()
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
            anyhow::ensure!(
                i.flow_control() != FlowControl::ConditionalBranch
                    || indices.contains_key(&i.near_branch_target()),
                "conditional branch leaves NT function at 0x{:X}",
                i.ip()
            );
            if let Some(&target) = indices.get(&i.near_branch_target()) {
                successors[n].push(target);
            }
        }
    }
    let mut regs = HashMap::from([(layout.proto_argument, Origin::Proto)]);
    if let Some(game) = layout.game_argument {
        regs.insert(game, Origin::Game);
    }
    let mut incoming = vec![None; instructions.len()];
    incoming[0] = Some(State {
        regs,
        stack: BTreeMap::new(),
        sp: Some(0),
    });
    let mut queue = VecDeque::from([0]);
    let mut info = InstructionInfoFactory::new();
    let mut steps = 0;
    while let Some(n) = queue.pop_front() {
        steps += 1;
        anyhow::ensure!(
            steps <= instructions.len().saturating_mul(128),
            "NT parser data flow exceeded convergence bound"
        );
        let mut state = incoming[n].clone().unwrap();
        transfer(
            &instructions[n],
            &mut state,
            &layout,
            &class_from_slot,
            &mut info,
        );
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
    let mut result = Vec::new();
    for (i, state) in instructions.iter().zip(incoming) {
        let Some(state) = state else { continue };
        if matches!(
            i.mnemonic(),
            Mnemonic::Mov | Mnemonic::Movss | Mnemonic::Movsd | Mnemonic::Movq | Mnemonic::Movd
        ) && i.op0_kind() == OpKind::Memory
            && i.op1_kind() == OpKind::Register
            && i.memory_index() == Register::None
            && state.get(i.memory_base()) == Some(Origin::Game)
            && let Some(Origin::Field(offset, width)) = state.get(i.op1_register())
            && width == i.memory_size().size()
            && width <= 8
        {
            result.push(FieldCopy {
                proto_offset: offset,
                game_offset: i.memory_displacement32(),
                instruction: i.ip() as usize,
            });
        }
    }
    Ok(result)
}

fn transfer(
    i: &Instruction,
    state: &mut State,
    layout: &ParserLayout,
    class_from_slot: &impl Fn(usize) -> Option<usize>,
    info: &mut InstructionInfoFactory,
) {
    let before = state.clone();
    let is_load = matches!(
        i.mnemonic(),
        Mnemonic::Mov
            | Mnemonic::Movzx
            | Mnemonic::Movsx
            | Mnemonic::Movsxd
            | Mnemonic::Movss
            | Mnemonic::Movsd
            | Mnemonic::Movq
            | Mnemonic::Movd
    );
    let value = if is_load {
        match i.op1_kind() {
            OpKind::Register => before
                .get(i.op1_register())
                .filter(|v| matches!(v, Origin::Field(..)) || i.op1_register().is_gpr64()),
            OpKind::Memory if i.is_ip_rel_memory_operand() && i.memory_size().size() == 8 => {
                class_from_slot(i.ip_rel_memory_address() as usize).map(Origin::Class)
            }
            OpKind::Memory if i.memory_index() == Register::None => {
                if before.get(i.memory_base()) == Some(Origin::Proto) && i.memory_size().size() <= 8
                {
                    Some(Origin::Field(
                        i.memory_displacement32(),
                        i.memory_size().size(),
                    ))
                } else if i.memory_base() == Register::RSP && i.memory_size().size() == 8 {
                    before.sp.and_then(|sp| {
                        before
                            .stack
                            .get(&(sp + i.memory_displacement64() as i64))
                            .copied()
                    })
                } else {
                    None
                }
            }
            _ => None,
        }
    } else {
        None
    };
    for used in info.info(i).used_registers() {
        if matches!(
            used.access(),
            OpAccess::Write | OpAccess::CondWrite | OpAccess::ReadWrite | OpAccess::ReadCondWrite
        ) {
            state.regs.remove(&used.register().full_register());
            if used.register().full_register() == Register::RSP {
                state.sp = None;
            }
        }
    }
    if matches!(i.mnemonic(), Mnemonic::Sub | Mnemonic::Add)
        && i.op0_register() == Register::RSP
        && matches!(
            i.op1_kind(),
            OpKind::Immediate8to64 | OpKind::Immediate32to64
        )
    {
        state.sp = before.sp.map(|sp| {
            sp + i.immediate(1) as i64 * if i.mnemonic() == Mnemonic::Sub { -1 } else { 1 }
        });
    }
    if i.mnemonic() == Mnemonic::Push {
        state.sp = before.sp.map(|sp| sp - 8);
        if let Some(sp) = state.sp {
            state.stack.remove(&sp);
            if let Some(v) = before.get(i.op0_register()) {
                state.stack.insert(sp, v);
            }
        }
    } else if i.mnemonic() == Mnemonic::Pop {
        if let Some(sp) = before.sp {
            if let Some(v) = before.stack.get(&sp) {
                state.regs.insert(i.op0_register().full_register(), *v);
            }
            state.stack.remove(&sp);
        }
        state.sp = before.sp.map(|sp| sp + 8);
    }
    if is_load
        && i.op0_kind() == OpKind::Register
        && let Some(v) = value
    {
        // Partial pointer writes cannot preserve object identity.
        if match v {
            Origin::Field(_, width) => {
                i.op0_register().size() >= width
                    && !matches!(
                        i.op0_register(),
                        Register::AH | Register::BH | Register::CH | Register::DH
                    )
            }
            _ => i.op0_register().is_gpr64(),
        } {
            state.regs.insert(i.op0_register().full_register(), v);
        }
    }
    if i.op0_kind() == OpKind::Memory && i.memory_base() == Register::RSP {
        if i.memory_index() != Register::None {
            state.stack.clear();
        } else if let Some(sp) = before.sp {
            let start = sp + i.memory_displacement64() as i64;
            let end = start + i.memory_size().size() as i64;
            state
                .stack
                .retain(|offset, _| *offset + 8 <= start || *offset >= end);
            if i.mnemonic() == Mnemonic::Mov
                && i.memory_size().size() == 8
                && let Some(v) = value
            {
                state.stack.insert(start, v);
            }
        }
    }
    if i.op0_kind() == OpKind::Memory
        && i.memory_base() != Register::RSP
        && !matches!(
            before.get(i.memory_base()),
            Some(Origin::Proto | Origin::Game)
        )
    {
        state.stack.clear();
    }
    if matches!(
        i.flow_control(),
        FlowControl::Call | FlowControl::IndirectCall
    ) {
        state.sp = before.sp;
        let allocated = super::super::native::direct_target(i) == Some(layout.allocation)
            && before.get(Register::RCX) == Some(Origin::Class(layout.game_class));
        for reg in [
            Register::RAX,
            Register::RCX,
            Register::RDX,
            Register::R8,
            Register::R9,
            Register::R10,
            Register::R11,
            Register::XMM0,
            Register::XMM1,
            Register::XMM2,
            Register::XMM3,
            Register::XMM4,
            Register::XMM5,
        ] {
            state.regs.remove(&reg.full_register());
        }
        state.stack.clear();
        if allocated {
            state.regs.insert(Register::RAX, Origin::Game);
        }
    }
}
