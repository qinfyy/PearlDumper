use std::collections::{BTreeSet, HashMap};

use anyhow::{Context, Result, ensure};
use iced_x86::{Decoder, DecoderOptions, Instruction, Mnemonic, OpKind, Register};
use serde::Serialize;
use utils::pe::FunctionTable;

#[derive(Debug, Serialize)]
pub struct HeaderOffset {
    pub member: usize,
    pub add: u32,
}

#[derive(Debug, Serialize)]
pub struct Layout {
    pub initializer: usize,
    pub range_entry: usize,
    pub header_global: usize,
    pub data_global: usize,
    pub usage_global: usize,
    pub lists: HeaderOffset,
    pub pairs: HeaderOffset,
    pub tables: [Option<usize>; 8],
}

pub fn discover(image: &[u8], functions: &FunctionTable) -> Result<Layout> {
    let mut candidates = BTreeSet::new();
    let key = 0x8CD81660EE8u64.to_le_bytes();
    for (rva, bytes) in image.windows(8).enumerate() {
        if bytes == key
            && let Some(range) = functions.containing(rva)
        {
            candidates.insert(range.start as usize);
        }
    }
    let mut accepted = Vec::new();
    let mut rejected = Vec::new();
    for rva in candidates {
        let range = functions.containing(rva).unwrap().range();
        match inspect(image, range.clone()) {
            Ok(layout) => accepted.push(layout),
            Err(error) => rejected.push(format!("0x{rva:X}: {error:#}")),
        }
    }
    ensure!(
        accepted.len() == 1,
        "expected one metadata initializer, found {}; rejected: {}",
        accepted.len(),
        rejected.join("; ")
    );
    Ok(accepted.remove(0))
}

fn instructions(image: &[u8], start: usize, end: usize) -> Result<Vec<Instruction>> {
    Ok(Decoder::with_ip(
        64,
        image.get(start..end).context("code outside image")?,
        start as u64,
        DecoderOptions::NONE,
    )
    .into_iter()
    .collect())
}

fn inspect(image: &[u8], range: std::ops::Range<usize>) -> Result<Layout> {
    let code = instructions(image, range.start, range.end)?;
    let range_entry = range_entry(image, &range, &code)?;
    // These identify the supported decoder algorithm, not a client's addresses or table layout.
    for constant in [
        0x8CD81660EE8,
        0x7AFCE30F,
        0xEF048154,
        0x9174AED3,
        0x87C3,
        0x5FA3FAD3,
        0x334FB2BA,
        0x454D10D89E6A02A,
        0x102533D7,
        0x58972D9863A807,
        0xAB3A6CB5,
        0x6907AB9A,
        0x1FFFFFFF,
    ] {
        ensure!(
            code.iter().any(|i| (0..i.op_count())
                .any(|op| is_immediate(i.op_kind(op)) && i.immediate(op) == constant)),
            "unsupported usage decoder, missing constant 0x{constant:X}"
        );
    }
    let prefix = code
        .iter()
        .position(|i| {
            i.op_count() == 2
                && i.op1_kind() == OpKind::Immediate64
                && i.immediate64() == 0x8CD81660EE8
        })
        .context("missing list decoder")?;
    let mut headers = Vec::new();
    for w in code[..prefix].windows(3) {
        if w[0].mnemonic() == Mnemonic::Mov
            && w[0].is_ip_rel_memory_operand()
            && w[0].op0_register().is_gpr64()
            && w[1].mnemonic() == Mnemonic::Mov
            && w[1].op1_kind() == OpKind::Immediate32
            && w[2].mnemonic() == Mnemonic::Add
            && w[2].op0_register() == w[1].op0_register()
            && w[2].op1_kind() == OpKind::Memory
            && w[2].memory_base() == w[0].op0_register()
        {
            headers.push((
                w[0].ip_rel_memory_address() as usize,
                w[0].op0_register(),
                HeaderOffset {
                    member: w[2].memory_displacement64() as usize,
                    add: w[1].immediate32(),
                },
            ));
        }
    }
    ensure!(headers.len() == 1, "ambiguous metadata header reference");
    let (header_global, header_register, lists) = headers.remove(0);
    ensure!(
        header_register == Register::RDX,
        "unsupported range header register"
    );
    let data_globals: BTreeSet<_> = code[..prefix]
        .iter()
        .filter(|i| {
            i.mnemonic() == Mnemonic::Mov
                && i.op0_register().is_gpr64()
                && i.is_ip_rel_memory_operand()
                && i.ip_rel_memory_address() as usize != header_global
        })
        .map(|i| i.ip_rel_memory_address() as usize)
        .collect();
    ensure!(data_globals.len() == 1, "ambiguous metadata data pointer");
    let data_global = *data_globals.first().unwrap();
    ensure!(
        code[..prefix].iter().any(|i| i.mnemonic() == Mnemonic::Mov
            && i.op0_register() == Register::R14
            && i.is_ip_rel_memory_operand()
            && i.ip_rel_memory_address() as usize == data_global),
        "unsupported range data register"
    );
    let mut switches = Vec::new();
    for (index, w) in code.windows(4).enumerate() {
        if w[0].mnemonic() == Mnemonic::Lea
            && w[0].is_ip_rel_memory_operand()
            && w[1].mnemonic() == Mnemonic::Movsxd
            && w[1].op1_kind() == OpKind::Memory
            && w[1].memory_base() == w[0].op0_register()
            && w[1].memory_index_scale() == 4
            && w[2].mnemonic() == Mnemonic::Add
            && w[2].op0_register() == w[1].op0_register()
            && w[2].op1_register() == w[0].op0_register()
            && w[3].mnemonic() == Mnemonic::Jmp
            && w[3].op0_register() == w[1].op0_register()
        {
            switches.push((
                index,
                w[0].ip_rel_memory_address() as usize,
                w[1].memory_displacement64() as i64,
            ));
        }
    }
    ensure!(switches.len() == 1, "ambiguous usage switch");
    let (switch, base, displacement) = switches[0];
    ensure!(
        code[..switch].iter().any(|i| i.mnemonic() == Mnemonic::Cmp
            && is_immediate(i.op1_kind())
            && i.immediate(1) == 7),
        "usage switch has no 0..7 bound"
    );
    let mut pair_offsets = Vec::new();
    for w in code[prefix..switch].windows(3) {
        if w[0].mnemonic() == Mnemonic::Mov
            && w[0].op1_kind() == OpKind::Memory
            && w[0].memory_base() == header_register
            && w[0].memory_size().size() == 4
            && w[1].mnemonic() == Mnemonic::Mov
            && w[1].op1_kind() == OpKind::Immediate32
            && w[2].mnemonic() == Mnemonic::Add
            && w[2].op0_register() == w[0].op0_register()
            && w[2].op1_register() == w[1].op0_register()
        {
            pair_offsets.push(HeaderOffset {
                member: w[0].memory_displacement64() as usize,
                add: w[1].immediate32(),
            });
        }
    }
    ensure!(pair_offsets.len() == 1, "ambiguous usage pair offset");
    let table = base
        .checked_add_signed(displacement as isize)
        .context("switch table overflow")?;
    let mut globals = BTreeSet::new();
    let mut tables = [None; 8];
    for kind in [0, 1, 3, 4, 5, 6, 7] {
        let relative = i32::from_le_bytes(
            image
                .get(table + kind * 4..table + kind * 4 + 4)
                .context("truncated switch table")?
                .try_into()?,
        );
        let target = base
            .checked_add_signed(relative as isize)
            .context("case RVA overflow")?;
        ensure!(range.contains(&target), "usage case outside initializer");
        let branch = instructions(image, target, range.end)?;
        let getter = branch
            .iter()
            .take(12)
            .collect::<Vec<_>>()
            .windows(2)
            .find_map(|w| {
                (w[0].mnemonic() == Mnemonic::Mov
                    && w[0].is_ip_rel_memory_operand()
                    && w[1].mnemonic() == Mnemonic::Mov
                    && w[1].op1_kind() == OpKind::Memory
                    && w[1].memory_base() == w[0].op0_register()
                    && w[1].memory_index() == Register::None)
                    .then(|| {
                        (
                            w[0].ip_rel_memory_address() as usize,
                            w[1].memory_displacement64() as usize,
                        )
                    })
            })
            .with_context(|| format!("missing table load in usage case {kind}"))?;
        ensure!(
            getter.1 <= 0x200 && getter.1.is_multiple_of(8),
            "invalid usage table member"
        );
        globals.insert(getter.0);
        tables[kind] = Some(getter.1);
    }
    ensure!(
        globals.len() == 1 && tables[3] == tables[6],
        "inconsistent usage tables"
    );
    for global in [header_global, data_global, *globals.first().unwrap()] {
        ensure!(
            global.checked_add(8).is_some_and(|end| end <= image.len()),
            "global outside image"
        );
    }
    Ok(Layout {
        initializer: range.start,
        range_entry,
        header_global,
        data_global,
        usage_global: *globals.first().unwrap(),
        lists,
        pairs: pair_offsets.remove(0),
        tables,
    })
}

fn range_entry(
    image: &[u8],
    range: &std::ops::Range<usize>,
    code: &[Instruction],
) -> Result<usize> {
    // This is an ABI contract, not a client address: the adapter must reproduce
    // the exact frame consumed by the original loop and native unwind records.
    const FRAME: &[u8] = &[
        0x55, 0x41, 0x57, 0x41, 0x56, 0x41, 0x55, 0x41, 0x54, 0x56, 0x57, 0x53, 0x48, 0x83, 0xec,
        0x38, 0x48, 0x8d, 0x6c, 0x24, 0x30, 0x48, 0xc7, 0x45, 0x00, 0xfe, 0xff, 0xff, 0xff,
    ];
    ensure!(
        image.get(range.start..range.start + FRAME.len()) == Some(FRAME),
        "unsupported metadata range frame"
    );
    let mut entries = BTreeSet::new();
    for w in code.windows(3) {
        if w[0].mnemonic() != Mnemonic::Inc
            || w[0].op0_register() != Register::R13D
            || w[1].mnemonic() != Mnemonic::Dec
            || w[1].op0_register() != Register::R12
            || w[2].mnemonic() != Mnemonic::Je
        {
            continue;
        }
        let entry = w[2].next_ip() as usize;
        let exit = w[2].near_branch_target();
        let loop_at = code
            .iter()
            .position(|i| i.ip() as usize == entry)
            .context("missing range loop")?;
        let loop_code = &code[loop_at..];
        ensure!(
            loop_code[0].mnemonic() == Mnemonic::Mov
                && loop_code[0].op0_register() == Register::ECX
                && loop_code[0].op1_register() == Register::R13D,
            "unsupported metadata range cursor"
        );
        let exit_at = code
            .iter()
            .position(|i| i.ip() == exit)
            .context("missing range exit")?;
        let exit_code = code
            .get(exit_at..exit_at + 13)
            .context("truncated range exit")?;
        ensure!(
            exit_code[0].mnemonic() == Mnemonic::Xor
                && exit_code[0].op0_register() == Register::ECX
                && exit_code[0].op1_register() == Register::ECX
                && exit_code[1].mnemonic() == Mnemonic::Call
                && exit_code[1].is_ip_rel_memory_operand()
                && exit_code[2].mnemonic() == Mnemonic::Nop
                && exit_code[3].mnemonic() == Mnemonic::Add
                && exit_code[3].op0_register() == Register::RSP
                && is_immediate(exit_code[3].op1_kind())
                && exit_code[3].immediate(1) == 0x38
                && exit_code[12].mnemonic() == Mnemonic::Ret,
            "unsupported metadata range epilogue"
        );
        for (i, register) in [
            Register::RBX,
            Register::RDI,
            Register::RSI,
            Register::R12,
            Register::R13,
            Register::R14,
            Register::R15,
            Register::RBP,
        ]
        .into_iter()
        .enumerate()
        {
            ensure!(
                exit_code[4 + i].mnemonic() == Mnemonic::Pop
                    && exit_code[4 + i].op0_register() == register,
                "unsupported metadata range restore"
            );
        }
        ensure!(
            code.iter().any(|i| i.mnemonic() == Mnemonic::Jmp
                && i.op0_kind() == OpKind::NearBranch64
                && i.near_branch_target() as usize == entry),
            "range loop has no native entry"
        );
        entries.insert(entry);
    }
    ensure!(entries.len() == 1, "no unique metadata range loop");
    Ok(*entries.first().unwrap())
}

fn is_immediate(kind: OpKind) -> bool {
    matches!(
        kind,
        OpKind::Immediate8
            | OpKind::Immediate32
            | OpKind::Immediate64
            | OpKind::Immediate8to32
            | OpKind::Immediate8to64
            | OpKind::Immediate32to64
    )
}

pub fn decode_lists(bytes: &[u8]) -> Result<Vec<u32>> {
    ensure!(
        bytes.len() >= 8 && bytes.len().is_multiple_of(4),
        "invalid usage list span"
    );
    let mut starts = Vec::with_capacity(bytes.len() / 4);
    for (index, raw) in bytes.chunks_exact(4).enumerate() {
        let key = ((0x8CD81660EE8u64.wrapping_mul(index as u64) >> 17).wrapping_add(0x7AFCE30F))
            as u32
            ^ 0xEF048154;
        let start = u32::from_le_bytes(raw.try_into()?)
            .wrapping_add(key)
            .wrapping_sub(0x6E8B512D);
        ensure!(
            (index != 0 || start == 0) && starts.last().is_none_or(|previous| *previous <= start),
            "invalid usage list boundary at index {index}"
        );
        starts.push(start);
    }
    Ok(starts)
}

#[derive(Debug, Serialize)]
pub struct UsageSlot {
    pub kind: usize,
    pub source: u32,
    pub index: u32,
}

pub fn decode_pairs(bytes: &[u8]) -> Result<Vec<UsageSlot>> {
    ensure!(bytes.len().is_multiple_of(8), "invalid usage pair span");
    let mut seen = HashMap::new();
    let mut slots = Vec::new();
    for (index, row) in bytes.chunks_exact(8).enumerate() {
        let a = (0x87C3u64.wrapping_mul(index as u64) ^ 0x5FA3FAD3)
            .wrapping_mul(0x334FB2BA)
            .wrapping_add(0x454D10D89E6A02A)
            >> 14;
        let key = (a.wrapping_mul(0x102533D7).wrapping_add(0x58972D9863A807) >> 23) as u32;
        let high = u32::from_le_bytes(row[4..].try_into()?)
            .wrapping_sub(key)
            .wrapping_sub(0x54C5934B);
        let low = (u32::from_le_bytes(row[..4].try_into()?) ^ 0x6907AB9A).wrapping_sub(key);
        let kind = (high >> 29) as usize;
        if kind == 2 {
            continue;
        }
        let source = high & 0x1FFFFFFF;
        let index = if matches!(kind, 0 | 7) {
            low & 0xFFFFFF
        } else {
            low
        };
        let table_kind = if kind == 6 { 3 } else { kind };
        if let Some(old) = seen.insert((table_kind, index), (kind, source, low)) {
            ensure!(
                old == (kind, source, low),
                "conflicting metadata slot {table_kind}:{index}"
            );
        } else {
            slots.push(UsageSlot {
                kind,
                source,
                index,
            });
        }
    }
    Ok(slots)
}
