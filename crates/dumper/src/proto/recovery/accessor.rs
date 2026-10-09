use iced_x86::{Decoder, DecoderOptions, Mnemonic, OpKind, Register};

pub(crate) fn storage_name(raw: &str, accessors: &[String]) -> String {
    use super::super::output::snake_field;
    let clean = raw
        .strip_prefix('<')
        .and_then(|s| s.strip_suffix(">k__BackingField"))
        .unwrap_or(raw);
    // The caller supplies only direct, type-checked accessors of this exact slot.
    if let Some(name) = clean.strip_prefix('_').filter(|s| !s.starts_with('_')) {
        let name = snake_field(name);
        if accessors.contains(&name) {
            return name;
        }
    }
    snake_field(clean)
}

// Only a load of this.field into the ABI return register followed by RET proves
// a direct accessor. Calls, branches and arithmetic require separate evidence.
pub(crate) fn direct_field(bytes: &[u8]) -> Option<(usize, usize)> {
    let mut field = None;
    for i in Decoder::new(64, bytes, DecoderOptions::NONE)
        .into_iter()
        .take(16)
    {
        if i.is_invalid() {
            return None;
        }
        match i.mnemonic() {
            Mnemonic::Nop => continue,
            Mnemonic::Ret if i.op_count() == 0 => return field,
            Mnemonic::Mov
            | Mnemonic::Movzx
            | Mnemonic::Movsx
            | Mnemonic::Movsxd
            | Mnemonic::Movss
            | Mnemonic::Movsd => {
                let width = i.memory_size().size();
                let valid_return = match i.mnemonic() {
                    Mnemonic::Movss => i.op0_register() == Register::XMM0 && width == 4,
                    Mnemonic::Movsd => i.op0_register() == Register::XMM0 && width == 8,
                    _ => {
                        matches!(
                            i.op0_register(),
                            Register::RAX | Register::EAX | Register::AX | Register::AL
                        ) && i.op0_register().size() >= width
                    }
                };
                if field.is_some()
                    || i.op0_kind() != OpKind::Register
                    || i.op1_kind() != OpKind::Memory
                    || i.memory_base() != Register::RCX
                    || i.memory_index() != Register::None
                    || !matches!(width, 1 | 2 | 4 | 8)
                    || !valid_return
                {
                    return None;
                }
                field = Some((i.memory_displacement64() as usize, width));
            }
            _ => return None,
        }
    }
    None
}
