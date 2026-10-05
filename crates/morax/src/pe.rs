use std::{fs, path::Path};

use rayon::prelude::*;

use crate::error::Result;

pub struct Pe {
    data: Vec<u8>,
    sections: Vec<Section>,
    image_base: u64,
}

struct Section {
    virtual_address: u32,
    virtual_size: u32,
    raw_offset: u32,
    raw_size: u32,
}

impl Pe {
    pub fn read(path: &Path) -> Result<Self> {
        Self::from_bytes(fs::read(path)?)
    }

    pub fn from_bytes(data: Vec<u8>) -> Result<Self> {
        if data.first_chunk::<2>() != Some(b"MZ") {
            return Err("not a PE image".into());
        }
        let pe = read_u32(&data, 0x3C)? as usize;
        if data.get(pe..pe + 4) != Some(b"PE\0\0") {
            return Err("bad PE signature".into());
        }

        let coff = pe + 4;
        let section_count = read_u16(&data, coff + 2)? as usize;
        let optional = coff + 20;
        let section_table = optional + read_u16(&data, coff + 16)? as usize;
        let image_base = match read_u16(&data, optional)? {
            0x20B => read_u64(&data, optional + 24)?,
            0x10B => read_u32(&data, optional + 28)? as u64,
            magic => return Err(format!("unsupported optional-header magic 0x{magic:X}").into()),
        };

        let sections = (0..section_count)
            .map(|i| {
                let s = section_table + i * 40;
                Ok(Section {
                    virtual_size: read_u32(&data, s + 8)?,
                    virtual_address: read_u32(&data, s + 12)?,
                    raw_size: read_u32(&data, s + 16)?,
                    raw_offset: read_u32(&data, s + 20)?,
                })
            })
            .collect::<Result<_>>()?;

        Ok(Self {
            data,
            sections,
            image_base,
        })
    }

    pub fn image_base(&self) -> u64 {
        self.image_base
    }

    pub fn contains_rva(&self, rva: u64, len: u64) -> bool {
        self.sections.iter().any(|section| {
            let start = u64::from(section.virtual_address);
            let end = start + u64::from(section.virtual_size.max(section.raw_size));
            rva >= start && rva.checked_add(len).is_some_and(|limit| limit <= end)
        })
    }

    pub fn rd8(&self, rva: u32) -> Result<u8> {
        read_u8(self.rd_bytes(rva, 1)?, 0)
    }
    pub fn rd16(&self, rva: u32) -> Result<u16> {
        read_u16(self.rd_bytes(rva, 2)?, 0)
    }
    pub fn rd32(&self, rva: u32) -> Result<u32> {
        read_u32(self.rd_bytes(rva, 4)?, 0)
    }
    pub fn rd64(&self, rva: u32) -> Result<u64> {
        read_u64(self.rd_bytes(rva, 8)?, 0)
    }

    pub fn pointer(&self, rva: u32) -> Result<u32> {
        let va = self.rd64(rva)?;
        va.checked_sub(self.image_base)
            .and_then(|offset| u32::try_from(offset).ok())
            .filter(|&target| {
                self.sections.iter().any(|s| {
                    target >= s.virtual_address
                        && target - s.virtual_address < s.virtual_size.max(s.raw_size)
                })
            })
            .ok_or_else(|| {
                format!(
                    "pointer at RVA 0x{rva:X} has target VA 0x{va:X} outside image base 0x{:X}",
                    self.image_base
                )
                .into()
            })
    }

    pub fn rd_bytes(&self, rva: u32, len: usize) -> Result<&[u8]> {
        let section = self
            .sections
            .iter()
            .find(|s| {
                rva >= s.virtual_address
                    && rva - s.virtual_address < s.raw_size
                    && len <= (s.raw_size - (rva - s.virtual_address)) as usize
            })
            .ok_or_else(|| {
                format!("RVA range 0x{rva:X}+0x{len:X} is not backed by one PE section")
            })?;
        let offset = section.raw_offset as usize + (rva - section.virtual_address) as usize;
        self.data
            .get(offset..offset.checked_add(len).ok_or("PE read length overflow")?)
            .ok_or_else(|| "out of range".into())
    }

    pub fn offset(&self, rva: u32) -> Result<usize> {
        self.sections
            .iter()
            .find(|s| rva >= s.virtual_address && rva - s.virtual_address < s.raw_size)
            .map(|s| (s.raw_offset + rva - s.virtual_address) as usize)
            .ok_or_else(|| format!("RVA 0x{rva:X} is outside the image").into())
    }

    pub fn scan(&self, pattern: &[Option<u8>]) -> Vec<u32> {
        let mut hits = Vec::new();
        for s in &self.sections {
            let start = s.raw_offset as usize;
            let end = (start + s.raw_size as usize).min(self.data.len());
            if pattern.is_empty() || end < start + pattern.len() {
                continue;
            }
            hits.par_extend(
                (start..=end - pattern.len())
                    .into_par_iter()
                    .filter_map(|o| {
                        pattern
                            .iter()
                            .zip(&self.data[o..])
                            .all(|(p, &b)| p.is_none_or(|x| x == b))
                            .then(|| s.virtual_address + (o - start) as u32)
                    }),
            );
        }
        hits
    }
}

macro_rules! read_le {
    ($($name:ident: $ty:ty),* $(,)?) => {$(
        pub fn $name(data: &[u8], offset: usize) -> Result<$ty> {
            const N: usize = std::mem::size_of::<$ty>();
            let end = offset.checked_add(N).ok_or("read offset overflow")?;
            data.get(offset..end)
                .map(|b| <$ty>::from_le_bytes(b.try_into().unwrap()))
                .ok_or_else(|| format!("read 0x{offset:X}+{N} exceeds buffer length 0x{:X}", data.len()).into())
        }
    )*};
}

read_le!(read_u8: u8, read_u16: u16, read_u32: u32, read_i32: i32, read_i64: i64, read_u64: u64);
