use std::{
    fs::File,
    io::{BufWriter, Write},
    time::{Duration, Instant},
};

use anyhow::Context;
use indexmap::IndexMap;
use serde::ser::{SerializeMap, Serializer};

use crate::vm::method::Il2CppMethod;

pub fn write(path: &str, methods: &IndexMap<String, Il2CppMethod>) -> anyhow::Result<()> {
    let started = Instant::now();
    let mut last_progress = started;
    let partial = format!("{path}.partial");
    log::info!("Writing {path}: {} method addresses", methods.len());
    let file = File::create(&partial).with_context(|| format!("creating {partial}"))?;
    let mut writer = BufWriter::new(file);
    let entries = methods.iter().enumerate().map(|(index, (name, method))| {
        if index == 0 || last_progress.elapsed() >= Duration::from_secs(2) {
            log::info!(
                "Writing {path}: {index}/{} methods, {:.1}s elapsed; next={name}, MethodInfo=0x{:X}",
                methods.len(),
                started.elapsed().as_secs_f64(),
                method.0
            );
            last_progress = Instant::now();
        }
        (name.as_str(), format!("0x{:X}", method.rva()))
    });
    serialize_entries(&mut writer, entries).with_context(|| format!("writing {partial}"))?;
    log::info!("Flushing {path}: {} method addresses", methods.len());
    writer
        .flush()
        .with_context(|| format!("flushing {partial}"))?;
    writer
        .get_ref()
        .sync_all()
        .with_context(|| format!("syncing {partial}"))?;
    drop(writer);
    std::fs::rename(&partial, path).with_context(|| format!("publishing {path}"))?;
    log::info!(
        "Wrote {path}: {} method addresses in {:.2}s",
        methods.len(),
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

fn serialize_entries<'a>(
    writer: &mut impl Write,
    entries: impl ExactSizeIterator<Item = (&'a str, String)>,
) -> serde_json::Result<()> {
    let mut serializer = serde_json::Serializer::pretty(writer);
    let mut map = serializer.serialize_map(Some(entries.len()))?;
    for (name, address) in entries {
        map.serialize_entry(name, &address)?;
    }
    map.end()
}
