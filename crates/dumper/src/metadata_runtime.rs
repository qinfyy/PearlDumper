use crate::metadata_scan::{self, Layout};
use anyhow::{Context, Result, ensure};
use std::{
    collections::HashSet,
    sync::{LazyLock, Mutex, OnceLock},
};

pub struct Plan {
    pub layout: Layout,
    list_starts: Vec<u32>,
}

static PLAN: OnceLock<Plan> = OnceLock::new();
static LOAD: Mutex<()> = Mutex::new(());
static INITIALIZED: LazyLock<Mutex<HashSet<u32>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

pub fn plan() -> Result<&'static Plan> {
    if let Some(plan) = PLAN.get() {
        return Ok(plan);
    }
    let _lock = LOAD
        .lock()
        .map_err(|_| anyhow::anyhow!("metadata discovery was interrupted"))?;
    if let Some(plan) = PLAN.get() {
        return Ok(plan);
    }
    let layout = metadata_scan::discover(
        utils::game_assembly_slice(),
        utils::scanner::game_assembly_functions()?,
    )?;
    let base = *il2cpp::GA_BASE;
    let header = unsafe { utils::memory::read::<usize>(base + layout.header_global) }?;
    let data = unsafe { utils::memory::read::<usize>(base + layout.data_global) }?;
    let offset = |entry: &metadata_scan::HeaderOffset| -> Result<usize> {
        let raw = unsafe {
            utils::memory::read::<u32>(
                header
                    .checked_add(entry.member)
                    .context("header overflow")?,
            )
        }?;
        Ok(raw.wrapping_add(entry.add) as usize)
    };
    let lists = offset(&layout.lists)?;
    let pairs = offset(&layout.pairs)?;
    let list_span = pairs
        .checked_sub(lists)
        .context("usage pairs precede lists")?;
    ensure!(
        (8..=64 * 1024 * 1024).contains(&list_span),
        "implausible usage list size {list_span}"
    );
    let bytes = unsafe {
        utils::memory::read_bytes(
            data.checked_add(lists).context("list address overflow")?,
            list_span,
        )
    }?;
    let list_starts = metadata_scan::decode_lists(&bytes)?;
    let pair_count = *list_starts.last().context("missing usage pair boundary")? as usize;
    ensure!(
        pair_count > 0 && pair_count <= 256 * 1024 * 1024 / 8,
        "implausible usage pair count {pair_count}"
    );
    // Proto resolves only lists referenced by verified native registration code.
    // No global usage-slot snapshot or bulk initializer is needed.
    log::info!(
        "Protocol metadata: initializer=0x{:X}, {} bounded usage lists; on-demand initialization only",
        layout.initializer,
        list_starts.len() - 1
    );
    PLAN.set(Plan {
        layout,
        list_starts,
    })
    .map_err(|_| anyhow::anyhow!("metadata plan already set"))?;
    PLAN.get().context("metadata plan was not stored")
}

pub fn initialize_list(index: u32) -> Result<()> {
    let plan = plan()?;
    let pair = plan
        .list_starts
        .get(index as usize..index as usize + 2)
        .with_context(|| format!("metadata usage list {index} is outside decoded bounds"))?;
    if pair[0] == pair[1] {
        return Ok(());
    }
    let mut initialized = INITIALIZED
        .lock()
        .map_err(|_| anyhow::anyhow!("metadata initialization was interrupted"))?;
    if initialized.contains(&index) {
        return Ok(());
    }
    let address = *il2cpp::GA_BASE + plan.layout.initializer;
    ensure!(
        utils::memory::is_executable(address),
        "metadata initializer is not executable"
    );
    let initialize: extern "C" fn(u32) = unsafe { std::mem::transmute(address) };
    microseh::try_seh(|| initialize(index))
        .map_err(|error| anyhow::anyhow!("metadata usage list {index} failed: {error:?}"))?;
    initialized.insert(index);
    Ok(())
}
