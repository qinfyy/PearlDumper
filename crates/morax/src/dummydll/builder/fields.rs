use crate::error::Result;

use super::super::tables::{self, Coded, Col};
use super::*;

impl AsmBuilder<'_> {
    pub(super) fn emit_fields(
        &mut self,
        type_index: usize,
        type_def: &crate::type_def::Il2CppTypeDefinition,
    ) -> Result<()> {
        for field in self.ctx.metadata.fields(type_index, type_def)? {
            let field_index = field.token & 0x00FF_FFFF;
            let constant = self.field_constant(field_index, field.flags)?;
            self.stats.fields += 1;
            // Initialized storage is not a CLI Constant row or a reconstructed FieldRVA.
            if field.flags & 0x0100 != 0
                || (constant.is_none()
                    && self
                        .ctx
                        .metadata
                        .field_default_values
                        .contains_key(&(field_index as usize)))
            {
                self.stats.initialized_field_data_not_emitted += 1;
            }
            let mut flags = u32::from(field.flags) & !0x8100;
            if constant.is_some() {
                flags |= 0x8000;
                self.stats.constants += 1;
            }

            let mut sig = vec![SIG_FIELD];
            self.encode_type(field.type_index, &mut sig)?;
            let name = self.string(&field.name);
            let sig_col = self.blob(&sig);
            let rid = self
                .tables
                .add(tables::FIELD, vec![Col::u16(flags), name, sig_col]);

            if let Some((element, value)) = constant {
                let blob = self.blob(&value);
                self.tables.add(
                    tables::CONSTANT,
                    vec![
                        Col::u16(u32::from(element)),
                        Col::coded(Coded::HasConstant, tables::FIELD, rid),
                        blob,
                    ],
                );
            }
            if field.offset > 0 {
                self.add_named_attribute(
                    tables::FIELD,
                    rid,
                    "FieldOffsetAttribute",
                    &[("Offset", format!("0x{:X}", field.offset))],
                )?;
            }
            self.add_token_attribute(tables::FIELD, rid, field.token)?;
        }
        Ok(())
    }

    fn field_constant(&self, field_index: u32, flags: u16) -> Result<Option<(u8, Vec<u8>)>> {
        if flags & 0x8040 == 0 {
            return Ok(None);
        }
        let Some(&(type_index, data_offset)) = self
            .ctx
            .metadata
            .field_default_values
            .get(&(field_index as usize))
        else {
            return Ok(None);
        };
        self.constant_blob(type_index, data_offset)
            .map(Some)
            .map_err(|error| error.context(format!("DummyDll constant field {field_index}")))
    }

    pub(super) fn constant_blob(&self, type_index: u32, data_offset: u32) -> Result<(u8, Vec<u8>)> {
        let (element, data) = self.ctx.metadata.constant_data(type_index, data_offset)?;
        let Some(data) = data else {
            return Ok((ET_CLASS, vec![0, 0, 0, 0]));
        };
        let value = match element {
            ET_STRING => String::from_utf8_lossy(data)
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect(),
            _ => data.to_vec(),
        };
        Ok((element, value))
    }
}
