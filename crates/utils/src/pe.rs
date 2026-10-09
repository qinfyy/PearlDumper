use std::ops::Range;

use anyhow::{Context, ensure};

pub struct SectionRange {
    pub range: Range<usize>,
    pub characteristics: u32,
}

pub fn mapped_sections(image: &[u8]) -> anyhow::Result<Vec<SectionRange>> {
    ensure!(image.get(..2) == Some(b"MZ"), "invalid DOS signature");
    let nt = read_u32(image, 0x3c)? as usize;
    ensure!(
        image.get(nt..nt + 4) == Some(b"PE\0\0"),
        "invalid PE signature"
    );
    ensure!(
        read_u16(image, nt + 4)? == 0x8664 && read_u16(image, nt + 24)? == 0x20b,
        "expected x64 PE32+ image"
    );
    let optional_size = read_u16(image, nt + 20)? as usize;
    ensure!(optional_size >= 64, "truncated PE optional header");
    let image_size = read_u32(image, nt + 80)? as usize;
    ensure!(image_size <= image.len(), "truncated mapped PE image");
    let count = read_u16(image, nt + 6)? as usize;
    ensure!((1..=96).contains(&count), "invalid PE section count");
    let table = nt + 24 + optional_size;
    let mut sections = Vec::with_capacity(count);
    for index in 0..count {
        let at = table + index * 40;
        let start = read_u32(image, at + 12)? as usize;
        let size = read_u32(image, at + 8)?.max(read_u32(image, at + 16)?) as usize;
        let end = start.checked_add(size).context("section range overflow")?;
        ensure!(end <= image_size, "section outside mapped image");
        sections.push(SectionRange {
            range: start..end,
            characteristics: read_u32(image, at + 36)?,
        });
    }
    Ok(sections)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FunctionRange {
    pub start: u32,
    pub end: u32,
}

impl FunctionRange {
    pub fn range(self) -> Range<usize> {
        self.start as usize..self.end as usize
    }
}

pub struct FunctionTable {
    entries: Vec<FunctionRange>,
    image_size: u32,
}

impl FunctionTable {
    pub fn from_mapped_pe(image: &[u8]) -> anyhow::Result<Self> {
        ensure!(image.get(..2) == Some(b"MZ"), "invalid DOS signature");
        let nt = read_u32(image, 0x3c)? as usize;
        ensure!(
            image.get(nt..nt + 4) == Some(b"PE\0\0"),
            "invalid PE signature"
        );
        let optional = nt.checked_add(24).context("PE header overflow")?;
        ensure!(read_u16(image, nt + 4)? == 0x8664, "expected x64 PE image");
        ensure!(
            read_u16(image, optional)? == 0x20b,
            "expected PE32+ optional header"
        );
        ensure!(
            read_u16(image, nt + 20)? >= 144,
            "truncated PE optional header"
        );
        ensure!(
            read_u32(image, optional + 108)? > 3,
            "missing PE exception directory"
        );
        let image_size = read_u32(image, optional + 56)?;
        ensure!(
            image_size as usize <= image.len(),
            "truncated mapped PE image"
        );

        // DataDirectory[EXCEPTION] is authoritative. In 4.6.51 the table is in .upx0,
        // while the section named .pdata does not contain RUNTIME_FUNCTION records.
        let directory = optional + 112 + 3 * 8;
        let rva = read_u32(image, directory)? as usize;
        let size = read_u32(image, directory + 4)? as usize;
        let end = rva
            .checked_add(size)
            .context("exception directory overflow")?;
        ensure!(
            end <= image_size as usize,
            "exception directory outside image"
        );
        Self::from_records(
            image
                .get(rva..end)
                .context("truncated exception directory")?,
            image_size,
        )
    }

    pub fn from_records(records: &[u8], image_size: u32) -> anyhow::Result<Self> {
        ensure!(
            !records.is_empty() && records.len().is_multiple_of(12),
            "invalid x64 exception table size"
        );
        let mut entries = Vec::with_capacity(records.len() / 12);
        let mut previous_end = 0;
        for row in records.chunks_exact(12) {
            let start = read_u32(row, 0)?;
            let end = read_u32(row, 4)?;
            let unwind = read_u32(row, 8)?;
            ensure!(
                start != 0 && start < end && end <= image_size,
                "invalid function range 0x{start:X}..0x{end:X}"
            );
            ensure!(
                start >= previous_end,
                "overlapping or unsorted PE functions at 0x{start:X}"
            );
            ensure!(unwind < image_size, "unwind record outside image");
            entries.push(FunctionRange { start, end });
            previous_end = end;
        }
        Ok(Self {
            entries,
            image_size,
        })
    }

    pub fn containing(&self, rva: usize) -> Option<FunctionRange> {
        let rva = u32::try_from(rva).ok()?;
        let index = self
            .entries
            .partition_point(|entry| entry.start <= rva)
            .checked_sub(1)?;
        let entry = self.entries[index];
        (rva < entry.end).then_some(entry)
    }

    pub fn entries(&self) -> &[FunctionRange] {
        &self.entries
    }

    // Only an absent unwind record is optional; invalid image ranges remain errors.
    pub fn candidate_bytes<'a>(
        &self,
        image: &'a [u8],
        rva: usize,
    ) -> anyhow::Result<Option<&'a [u8]>> {
        ensure!(
            image.len() >= self.image_size as usize,
            "truncated mapped PE image"
        );
        ensure!(
            rva < self.image_size as usize,
            "native function outside mapped image: 0x{rva:X}"
        );
        let Some(range) = self.containing(rva) else {
            return Ok(None);
        };
        Ok(Some(
            image
                .get(rva..range.end as usize)
                .context("native function outside mapped image")?,
        ))
    }

    pub fn function_bytes<'a>(&self, image: &'a [u8], rva: usize) -> anyhow::Result<&'a [u8]> {
        self.candidate_bytes(image, rva)?
            .with_context(|| format!("no native function contains RVA 0x{rva:X}"))
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> anyhow::Result<u16> {
    Ok(u16::from_le_bytes(
        bytes
            .get(offset..offset + 2)
            .context("truncated PE field")?
            .try_into()?,
    ))
}

fn read_u32(bytes: &[u8], offset: usize) -> anyhow::Result<u32> {
    Ok(u32::from_le_bytes(
        bytes
            .get(offset..offset + 4)
            .context("truncated PE field")?
            .try_into()?,
    ))
}
