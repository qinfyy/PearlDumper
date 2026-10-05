// Il2CppFieldDefaultValue
// fields = il2cpp_class_get_fields(v5, v26);
//     if ( !fields )
//       return v21 & 1;
//     v13 = (_QWORD *)fields;
//     v14 = (const char *)(*(_QWORD *)(fields + 16) - 0x72916D972C0FE1BFLL);
//     if ( strcmp("value__", v14) ) // this string
//     {
//       v15 = *v25;
//       v16 = strlen(v14);
//       *(_QWORD *)(v15 + 8 * v22 + 32) = sub_3944DE0(v14, v16);
//       v17 = strlen((const char *)(v13[2] - 0x72916D972C0FE1BFLL));
//       *(_QWORD *)(v15 + 8 * v22 + 32) = sub_3944DE0(v13[2] - 0x72916D972C0FE1BFLL, v17);
//       v11 = sub_39393D0(v5, v13); // here
use std::collections::HashMap;

use crate::pe::{Pe, read_i32};
use anyhow::{Result, ensure};

pub const ENTRY_SIZE: usize = 12;

pub struct RawFieldDefaultValue {
    pub type_index: i32,
    pub data_index: i32,
    pub field_index: i32,
}

pub fn decrypt(data: &[u8], entry: usize) -> Result<RawFieldDefaultValue> {
    Ok(RawFieldDefaultValue {
        type_index: read_i32(data, entry)?,
        data_index: read_i32(data, entry + 4)?,
        field_index: read_i32(data, entry + 8)?,
    })
}

pub fn load(
    global_data: &[u8],
    payload_offset: u32,
    field_default_values_offset: u32,
    count: u32,
) -> Result<HashMap<usize, (u32, u32)>> {
    let base = payload_offset as usize + field_default_values_offset as usize;
    let end = base + count as usize * ENTRY_SIZE;
    ensure!(
        end <= global_data.len(),
        "field default table 0x{base:X}..0x{end:X} exceeds metadata"
    );

    let mut map = HashMap::new();
    for index in 0..count as usize {
        let entry = base + index * ENTRY_SIZE;
        let raw = decrypt(global_data, entry)?;
        ensure!(
            raw.field_index >= 0 && raw.type_index >= -1 && raw.data_index >= -1,
            "invalid field default entry {index}: field={}, type={}, data={}",
            raw.field_index,
            raw.type_index,
            raw.data_index
        );
        let previous = map.insert(
            raw.field_index as usize,
            (raw.type_index as u32, raw.data_index as u32),
        );
        ensure!(
            previous.is_none(),
            "duplicate default value for field {} at entry {index}",
            raw.field_index
        );
    }
    Ok(map)
}

pub fn head_block(pe: &Pe, hdr: u32) -> Result<(u32, u32, u32)> {
    let field_default_values_offset = pe.rd32(hdr + 0x1FC)? ^ 0x6238CDB0; // *(_QWORD *)&v5 = qword_8E5B120 + (*(int *)(qword_8E5B118 + 0x1FC) ^ 0x6238CDB0LL);
    let field_and_parameter_default_value_data_offset =
        pe.rd32(hdr + 0x3C)?.wrapping_sub(0x6874185B);
    // Original DLL RVA 0x3EC318D..0x3EC31E9 truncates the span to whole records.
    let span = pe.rd32(hdr + 0x1DC)? ^ 0x3E0C72F0;
    Ok((
        field_default_values_offset,
        field_and_parameter_default_value_data_offset,
        span / ENTRY_SIZE as u32,
    ))
}

pub fn parameter_head_block(pe: &Pe, hdr: u32) -> Result<(u32, u32)> {
    // Original DLL 0x3F23DD0..0x3F23E89: span / 12, not a record count.
    Ok((
        pe.rd32(hdr + 0x4C)?.wrapping_add(0xE82C6008),
        (pe.rd32(hdr + 0x60)? ^ 0x2F5D2DD0) / ENTRY_SIZE as u32,
    ))
}

pub fn load_parameters(
    global_data: &[u8],
    payload_offset: u32,
    offset: u32,
    count: u32,
) -> Result<HashMap<usize, (u32, u32)>> {
    let base = payload_offset as usize + offset as usize;
    let end = base + count as usize * ENTRY_SIZE;
    ensure!(
        end <= global_data.len(),
        "parameter default table 0x{base:X}..0x{end:X} exceeds metadata"
    );
    let mut map = HashMap::new();
    for index in 0..count as usize {
        let entry = base + index * ENTRY_SIZE;
        // Unlike field defaults, the parameter index is the middle member.
        let type_index = read_i32(global_data, entry)?;
        let parameter_index = read_i32(global_data, entry + 4)?;
        let data_index = read_i32(global_data, entry + 8)?;
        ensure!(
            parameter_index >= 0 && type_index >= -1 && data_index >= -1,
            "invalid parameter default entry {index}: parameter={parameter_index}, type={type_index}, data={data_index}"
        );
        ensure!(
            map.insert(
                parameter_index as usize,
                (type_index as u32, data_index as u32)
            )
            .is_none(),
            "duplicate default value for parameter {parameter_index} at entry {index}"
        );
    }
    Ok(map)
}
