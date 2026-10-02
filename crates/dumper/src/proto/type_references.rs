use std::collections::HashSet;

use iced_x86::{Instruction, Mnemonic, OpKind};
use il2cpp::vm::class::Il2CppClass;

pub(super) struct TypeReferences {
    base: usize,
    size: usize,
    classes: HashSet<Il2CppClass>,
}

impl TypeReferences {
    pub fn new(base: usize, size: usize, classes: impl IntoIterator<Item = Il2CppClass>) -> Self {
        Self {
            base,
            size,
            classes: classes.into_iter().filter(|class| class.0 != 0).collect(),
        }
    }

    pub fn from_runtime() -> anyhow::Result<Self> {
        let classes = il2cpp::CLASS_TABLE_VEC
            .get()
            .ok_or_else(|| anyhow::anyhow!("IL2CPP class cache is not initialized"))?;
        anyhow::ensure!(!classes.is_empty(), "IL2CPP class cache is empty");
        Ok(Self::new(
            *il2cpp::GA_BASE,
            il2cpp::profile::supported()?.assembly.image_size as usize,
            classes.iter().copied(),
        ))
    }

    pub fn slot_address(&self, instruction: &Instruction) -> Option<usize> {
        if instruction.mnemonic() != Mnemonic::Mov
            || instruction.op0_kind() != OpKind::Register
            || !instruction.op0_register().is_gpr64()
            || instruction.op1_kind() != OpKind::Memory
            || !instruction.is_ip_rel_memory_operand()
        {
            return None;
        }
        let address = usize::try_from(instruction.ip_rel_memory_address()).ok()?;
        let offset = address.checked_sub(self.base)?;
        if offset.checked_add(size_of::<usize>())? > self.size {
            return None;
        }
        Some(address)
    }

    pub fn class_from_load(&self, instruction: &Instruction) -> Option<Il2CppClass> {
        let address = self.slot_address(instruction)?;
        // Metadata usage slots can change as the game initializes them; read the live value.
        let class = unsafe { utils::memory::read::<Il2CppClass>(address) }.ok()?;
        self.classes.contains(&class).then_some(class)
    }
}

#[cfg(test)]
mod tests {
    use iced_x86::{Decoder, DecoderOptions};

    use super::*;

    fn load(base: usize, slot: usize) -> Instruction {
        let displacement = i32::try_from(slot as i128 - (base + 7) as i128).unwrap();
        let mut bytes = vec![0x48, 0x8B, 0x0D];
        bytes.extend_from_slice(&displacement.to_le_bytes());
        Decoder::with_ip(64, &bytes, base as u64, DecoderOptions::NONE).decode()
    }

    #[test]
    fn resolves_known_class_without_script_type_infos() {
        let slots = [0usize, 0x1234, 0x1234, 0x4321];
        let base = slots.as_ptr() as usize;
        let class = Il2CppClass(0x1234);
        let references = TypeReferences::new(base, size_of_val(&slots), [class]);
        assert_eq!(
            references.class_from_load(&load(base, base + 8)),
            Some(class)
        );
        assert_eq!(
            references.class_from_load(&load(base, base + 16)),
            Some(class)
        );
        assert_eq!(references.class_from_load(&load(base, base)), None);
        assert_eq!(references.class_from_load(&load(base, base + 24)), None);
    }

    #[test]
    fn rejects_out_of_image_and_partial_pointer_slots() {
        let slots = [0x1234usize; 2];
        let base = slots.as_ptr() as usize;
        let references = TypeReferences::new(base, size_of_val(&slots), [Il2CppClass(0x1234)]);
        for address in [base - 8, base + 9, base + 16] {
            assert_eq!(references.class_from_load(&load(base, address)), None);
        }
    }

    #[test]
    fn rejects_unreadable_slots() {
        let references = TypeReferences::new(1, 4096, [Il2CppClass(0x1234)]);
        assert_eq!(references.class_from_load(&load(1, 8)), None);
    }

    #[test]
    fn rejects_non_pointer_loads() {
        let references = TypeReferences::new(1, 4096, [Il2CppClass(0x1234)]);
        for bytes in [
            &[0x48, 0x8B, 0x4C, 0x24, 0x08][..],    // mov rcx, [rsp+8]
            &[0x8B, 0x0D, 0x08, 0, 0, 0][..],       // mov ecx, [rip+8]
            &[0x48, 0x8D, 0x0D, 0x08, 0, 0, 0][..], // lea rcx, [rip+8]
            &[0x48, 0x89, 0x0D, 0x08, 0, 0, 0][..], // mov [rip+8], rcx
        ] {
            let instruction = Decoder::with_ip(64, bytes, 1, DecoderOptions::NONE).decode();
            assert_eq!(references.class_from_load(&instruction), None);
        }
    }

    #[test]
    fn reads_slots_live_instead_of_reusing_stale_values() {
        let mut slot = 0usize;
        let base = std::ptr::from_ref(&slot) as usize;
        let class = Il2CppClass(0x1234);
        let references = TypeReferences::new(base, size_of_val(&slot), [class]);
        let instruction = load(base, base);
        assert_eq!(references.class_from_load(&instruction), None);
        slot = class.0;
        assert_eq!(references.class_from_load(&instruction), Some(class));
        assert_eq!(slot, class.0);
    }
}
