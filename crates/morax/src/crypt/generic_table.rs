// Il2CppGenericMethodFunctionsDefinitions { specIndex: i32, fallbackIndex: i32, methodIndex: u16 }
// PATTERN = E8 ? ? ? ? 48 8B 4F 28 EB ?

use anyhow::Result;

use crate::pe::Pe;

pub const ENTRY_SIZE: usize = 12;
pub const FALLBACK_INDEX_OFF: usize = 0x4;
pub const METHOD_INDEX_OFF: usize = 0x8;
pub const SHARED_METHOD_INDEX: u16 = 0xFFFF;

// 4.6.51, original DLL RVA 0x3F20912..0x3F20957:
// entry+8 selects the primary table; 0xFFFF selects entry+4 in the secondary table.
pub const CODE_REGISTRATION_GENERIC_METHOD_POINTERS_OFF: u32 = 0x90;
pub const CODE_REGISTRATION_SECONDARY_POINTERS_OFF: u32 = 0x30;

// global metadata init under method_count hdr = Il2CppGlobalMetadataHeader
pub fn head_block(pe: &Pe, hdr: u32) -> Result<(u32, u32)> {
    let offset = pe.rd32(hdr + 0xEC)? ^ 0x67F701BA; // v626 = qword_8E5B120 + (*(int *)(qword_8E5B118 + 0xEC) ^ 0x67F701BALL);// generic table offset
    let count = (pe.rd32(hdr + 0x1E4)? ^ 0x720FEF70) / ENTRY_SIZE as u32; // v628 = (*(int *)(qword_8E5B118 + 484) ^ 0x720FEF70uLL) / 0xC;// generic table count
    Ok((offset, count))
}
