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
        self.class_from_slot(address)
    }

    pub fn class_from_slot(&self, address: usize) -> Option<Il2CppClass> {
        let offset = address.checked_sub(self.base)?;
        if offset.checked_add(size_of::<usize>())? > self.size {
            return None;
        }
        // Metadata usage slots can change as the game initializes them; read the live value.
        let class = unsafe { utils::memory::read::<Il2CppClass>(address) }.ok()?;
        self.classes.contains(&class).then_some(class)
    }
}
