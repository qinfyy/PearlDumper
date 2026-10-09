use iced_x86::{Decoder, DecoderOptions, Mnemonic, OpKind, Register};

// Evaluate only a tiny, call-free enum selector. Nothing is executed in-game.
// Unknown instructions, conditions, loops and out-of-range branches fail closed.
pub(super) fn selected_field(bytes: &[u8], ip: usize, value: i32) -> Option<usize> {
    let mut pc = ip as u64;
    let mut this_offset = 0i64;
    let mut equal = None;
    let mut field = None;
    let mut seen = std::collections::HashSet::new();
    for _ in 0..32 {
        if !seen.insert(pc) {
            return None;
        }
        let offset = usize::try_from(pc.checked_sub(ip as u64)?).ok()?;
        let i = Decoder::with_ip(64, bytes.get(offset..)?, pc, DecoderOptions::NONE).decode();
        if i.is_invalid() {
            return None;
        }
        pc = i.next_ip();
        match i.mnemonic() {
            Mnemonic::Nop => {}
            Mnemonic::Cmp
                if i.op0_kind() == OpKind::Register
                    && i.op0_register() == Register::EDX
                    && matches!(i.op1_kind(), OpKind::Immediate8to32 | OpKind::Immediate32) =>
            {
                equal = Some(value == i.immediate(1) as i32);
            }
            Mnemonic::Je | Mnemonic::Jne => {
                if equal? == (i.mnemonic() == Mnemonic::Je) {
                    pc = i.near_branch_target();
                }
            }
            Mnemonic::Jmp if i.op0_kind() == OpKind::NearBranch64 => pc = i.near_branch_target(),
            Mnemonic::Add
                if i.op0_kind() == OpKind::Register
                    && i.op0_register() == Register::RCX
                    && matches!(
                        i.op1_kind(),
                        OpKind::Immediate8to64 | OpKind::Immediate32to64
                    ) =>
            {
                this_offset = this_offset.checked_add(i.immediate(1) as i64)?;
                equal = None;
            }
            Mnemonic::Lea
                if i.op0_register() == Register::RCX
                    && i.memory_base() == Register::RCX
                    && i.memory_index() == Register::None =>
            {
                this_offset = this_offset.checked_add(i.memory_displacement64() as i64)?;
            }
            Mnemonic::Mov
                if i.op0_kind() == OpKind::Register
                    && i.op0_register() == Register::RAX
                    && i.op1_kind() == OpKind::Memory
                    && i.memory_base() == Register::RCX
                    && i.memory_index() == Register::None
                    && i.memory_size().size() == 8
                    && field.is_none() =>
            {
                field = Some(
                    usize::try_from(this_offset.checked_add(i.memory_displacement64() as i64)?)
                        .ok()?,
                );
            }
            Mnemonic::Ret if i.op_count() == 0 => return field,
            _ => return None,
        }
    }
    None
}
