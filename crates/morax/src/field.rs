use crate::crypt::attributes::FieldAttributes;
use crate::crypt::field as field_crypt;
use crate::crypt::token;
use crate::error::Result;
use crate::metadata::Metadata;
use crate::pe::{read_i32, read_i64, read_u8, read_u16, read_u32, read_u64};
use crate::type_def::Il2CppTypeDefinition;

macro_rules! managed_float {
    ($v:expr) => {{
        let v = $v;
        if v.is_nan() {
            "NaN".to_owned()
        } else if v.is_infinite() {
            if v < 0.0 { "-Infinity" } else { "Infinity" }.to_owned()
        } else {
            serde_json::to_string(&v).unwrap_or_default()
        }
    }};
}

#[derive(Clone, Debug)]
pub struct Il2CppFieldDefinition {
    pub name: String,
    pub type_name: String,
    pub type_index: u32,
    pub flags: u16,
    pub offset: u32,
    pub token: u32,
    pub default_value: Option<String>,
}

impl Metadata {
    pub fn fields(
        &self,
        type_index: usize,
        type_def: &Il2CppTypeDefinition,
    ) -> Result<Vec<Il2CppFieldDefinition>> {
        let Some(field_start) = type_def.field_start else {
            return Ok(Vec::new());
        };

        let mut fields = Vec::with_capacity(type_def.field_count);
        for local_index in 0..type_def.field_count {
            let field_index = field_start + local_index;
            let entry_offset = self.header.payload_offset as usize
                + self.header.fields_offset as usize
                + field_index * field_crypt::ENTRY_SIZE;
            let raw = field_crypt::decrypt(
                &self.global_data,
                entry_offset,
                field_start as u32,
                local_index as u32,
            )?;

            let name = self.decode_string(raw.name_index)?;
            let type_name = self.type_csharp_name(raw.type_index).map_err(|error| {
                error.context(format!(
                    "field {field_index} ({name}) type {}",
                    raw.type_index
                ))
            })?;
            let flags = self.il2cpp_type(raw.type_index)?.attrs;
            let offset = self
                .field_offset(type_index, local_index)
                .map_err(|error| error.context(format!("field {field_index} ({name}) offset")))?;

            let default_value = self
                .field_default_value(field_index, flags)
                .map_err(|error| {
                    error.context(format!("field {field_index} ({name}) default value"))
                })?;

            fields.push(Il2CppFieldDefinition {
                name,
                type_name,
                type_index: raw.type_index,
                flags,
                offset,
                token: token::field_token(field_index),
                default_value,
            });
        }
        Ok(fields)
    }

    fn field_default_value(&self, field_index: usize, flags: u16) -> Result<Option<String>> {
        if flags & FieldAttributes::Literal.bits() as u16 == 0 {
            return Ok(None);
        }
        let Some(&(type_index, data_offset)) = self.field_default_values.get(&field_index) else {
            return Err(format!("literal field {field_index} has no default record").into());
        };
        self.format_default_value(type_index, data_offset).map(Some)
    }

    pub(crate) fn format_default_value(&self, type_index: u32, data_offset: u32) -> Result<String> {
        let (kind, data) = self.constant_data(type_index, data_offset)?;
        let Some(data) = data else {
            return Ok("null".to_owned());
        };
        let value = match kind {
            0x02 => match read_u8(data, 0)? {
                0 => "false".to_owned(),
                1 => "true".to_owned(),
                value => return Err(format!("invalid boolean constant {value}").into()),
            },
            0x03 => format!("'\\u{:04X}'", read_u16(data, 0)?),
            0x04 => (read_u8(data, 0)? as i8).to_string(),
            0x05 => read_u8(data, 0)?.to_string(),
            0x06 => (read_u16(data, 0)? as i16).to_string(),
            0x07 => read_u16(data, 0)?.to_string(),
            0x08 => read_i32(data, 0)?.to_string(),
            0x09 => read_u32(data, 0)?.to_string(),
            0x0A => read_i64(data, 0)?.to_string(),
            0x0B => read_u64(data, 0)?.to_string(),
            0x0C => managed_float!(f32::from_bits(read_u32(data, 0)?)),
            0x0D => managed_float!(f64::from_bits(read_u64(data, 0)?)),
            0x0E => serde_json::to_string(&String::from_utf8_lossy(data))
                .map_err(|error| format!("escaping string constant: {error}"))?,
            _ => return Err(format!("unsupported constant kind 0x{kind:X}").into()),
        };
        Ok(value)
    }

    pub(crate) fn constant_data(
        &self,
        type_index: u32,
        data_offset: u32,
    ) -> Result<(u8, Option<&[u8]>)> {
        if data_offset == u32::MAX {
            return Ok((0x12, None));
        }
        let entry = self.il2cpp_type(type_index)?;
        let mut kind = entry.kind;
        if kind == 0x11 {
            let type_def = self.type_def(entry.data as usize)?;
            if !type_def.is_enum {
                return Err(format!("non-enum constant type {type_index}").into());
            }
            let field_start = type_def.field_start.ok_or("enum has no underlying field")?;
            let mut underlying = None;
            for local in 0..type_def.field_count {
                let offset = self.header.payload_offset as usize
                    + self.header.fields_offset as usize
                    + (field_start + local) * field_crypt::ENTRY_SIZE;
                let raw = field_crypt::decrypt(
                    &self.global_data,
                    offset,
                    field_start as u32,
                    local as u32,
                )?;
                let field_type = self.il2cpp_type(raw.type_index)?;
                if field_type.attrs & 0x50 == 0 {
                    if underlying.replace(field_type.kind).is_some() {
                        return Err("enum has multiple underlying fields".into());
                    }
                }
            }
            kind = underlying.ok_or("enum has no underlying field")?;
        }
        let base = self.header.payload_offset as usize
            + self.header.field_and_parameter_default_value_data_offset as usize
            + data_offset as usize;
        let mut start = base;
        let size = match kind {
            0x02 | 0x04 | 0x05 => 1,
            0x03 | 0x06 | 0x07 => 2,
            0x08 | 0x09 | 0x0C => 4,
            0x0A | 0x0B | 0x0D => 8,
            0x0E => {
                let length = read_i32(&self.global_data, base)?;
                if length == -1 {
                    return Ok((0x12, None));
                }
                if length < 0 {
                    return Err(format!("invalid string constant length {length}").into());
                }
                start += 4;
                length as usize
            }
            0x12 | 0x15 | 0x1C | 0x1D => {
                if read_u32(&self.global_data, base)? != 0 {
                    return Err(format!("non-null reference constant type {type_index}").into());
                }
                return Ok((0x12, None));
            }
            _ => {
                return Err(
                    format!("unsupported constant type {type_index} kind 0x{kind:X}").into(),
                );
            }
        };
        let end = start.checked_add(size).ok_or("constant length overflow")?;
        let bytes = self.global_data.get(start..end)
            .ok_or_else(|| format!("constant type {type_index} data 0x{data_offset:X}: 0x{start:X}..0x{end:X} exceeds metadata"))?;
        Ok((kind, Some(bytes)))
    }

    fn field_offset(&self, type_index: usize, local_field_index: usize) -> Result<u32> {
        let map_offset = self.header.payload_offset as usize
            + self.header.field_offset_map_offset as usize
            + type_index * 4;
        let selector = read_i32(&self.global_data, map_offset)?;
        if selector < 0 {
            return Ok(0);
        }

        let group_offset = self.header.payload_offset as usize
            + self.header.field_offset_group_offset as usize
            + selector as usize * 12;
        let offset_start = read_u32(&self.global_data, group_offset + 8)? as usize;

        let offset_entry = self.header.payload_offset as usize
            + self.header.field_offset_offset as usize
            + (offset_start + local_field_index) * 4;
        Ok(read_u32(&self.global_data, offset_entry)? & field_crypt::OFFSET_MASK)
    }
}
