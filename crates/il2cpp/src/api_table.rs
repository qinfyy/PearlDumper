use std::{collections::HashMap, sync::OnceLock};

use crate::vm::method::Il2CppMethod;
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiKind {
    Runtime,
    Native,
}

#[derive(Debug, Clone, Copy)]
pub struct ApiEntry {
    pub class: &'static str,
    pub member: &'static str,
    pub index: u32,
    pub kind: ApiKind,
    pub owner: &'static str,
    pub rust_fn: &'static str,
    pub parameter_count: usize,
    pub instance: bool,
}

impl ApiEntry {
    #[inline]
    pub fn signature(&self) -> String {
        format!("{}::{}{}", self.class, self.member, self.index)
    }
}

inventory::collect!(ApiEntry);

static RESOLVED_APIS: OnceLock<HashMap<String, Il2CppMethod>> = OnceLock::new();

#[derive(Debug, Deserialize)]
pub struct BindingProfile {
    pub version: String,
    pub assembly_sha256: String,
    pub bindings: Vec<Binding>,
}

#[derive(Debug, Deserialize)]
pub struct Binding {
    pub signature: String,
    pub class: String,
    pub member: String,
    pub parameter_count: usize,
    pub instance: bool,
    pub native: bool,
    pub rva: usize,
}

pub fn supported_bindings() -> anyhow::Result<BindingProfile> {
    Ok(serde_json::from_str(include_str!(
        "../../../profiles/4.6.51-bindings.json"
    ))?)
}

pub fn resolved_method(signature: &str) -> Option<Il2CppMethod> {
    RESOLVED_APIS.get()?.get(signature).copied()
}

pub fn validate_declarations(profile: &BindingProfile) -> anyhow::Result<()> {
    let client = crate::profile::supported()?;
    anyhow::ensure!(
        profile.version == client.version && profile.assembly_sha256 == client.assembly.sha256,
        "reflection bindings and client fingerprints do not match"
    );
    let mut signatures = std::collections::HashSet::new();
    for binding in &profile.bindings {
        anyhow::ensure!(
            signatures.insert(&binding.signature),
            "duplicate binding {}",
            binding.signature
        );
        anyhow::ensure!(
            binding.rva > 0 && binding.rva < client.assembly.image_size as usize,
            "invalid RVA for {}",
            binding.signature
        );
    }
    for entry in all_entries() {
        let signature = entry.signature();
        let binding = profile
            .bindings
            .iter()
            .find(|binding| binding.signature == signature)
            .ok_or_else(|| anyhow::anyhow!("missing verified binding {signature}"))?;
        anyhow::ensure!(
            binding.class == entry.class
                && binding.member == entry.member
                && binding.parameter_count == entry.parameter_count
                && binding.instance == entry.instance
                && binding.native == (entry.kind == ApiKind::Native),
            "binding declaration changed: {signature}"
        );
    }
    Ok(())
}

pub fn bind_profile() -> anyhow::Result<()> {
    if RESOLVED_APIS.get().is_some() {
        return Ok(());
    }
    let profile = supported_bindings()?;
    validate_declarations(&profile)?;
    let mut resolved = HashMap::new();
    log::info!(
        "Resolving {} exact reflection bindings",
        profile.bindings.len()
    );
    for (index, entry) in all_entries().enumerate() {
        let signature = entry.signature();
        let binding = profile
            .bindings
            .iter()
            .find(|binding| binding.signature == signature)
            .unwrap();
        log::info!(
            "Resolving binding {}/{}: {signature}, RVA=0x{:X}",
            index + 1,
            profile.bindings.len(),
            binding.rva
        );
        let class = crate::get_cached_class(&binding.class)
            .ok_or_else(|| anyhow::anyhow!("missing class {}", binding.class))?;
        let matches = class
            .get_methods()
            .into_iter()
            .filter(|method| method.get_name() == binding.member && method.rva() == binding.rva)
            .collect::<Vec<_>>();
        anyhow::ensure!(
            matches.len() == 1,
            "{signature}: expected one method at RVA 0x{:X}, found {}",
            binding.rva,
            matches.len()
        );
        let method = matches[0];
        anyhow::ensure!(
            crate::api::il2cpp_method_get_param_count(method) as usize == binding.parameter_count,
            "{signature}: runtime parameter count differs from dump"
        );
        // Method flags in the supplied dump are obfuscated; only check arity and the exact body here.
        method.checked_va()?;
        resolved.insert(signature, method);
    }
    log::info!(
        "Resolved {} exact reflection bindings for {}",
        resolved.len(),
        profile.version
    );
    RESOLVED_APIS
        .set(resolved)
        .map_err(|_| anyhow::anyhow!("reflection bindings initialized twice"))
}

#[inline]
pub fn all_entries() -> impl Iterator<Item = &'static ApiEntry> {
    inventory::iter::<ApiEntry>()
}

pub fn signature_of(owner: &str, rust_fn: &str) -> Option<String> {
    all_entries()
        .find(|e| e.owner == owner && e.rust_fn == rust_fn)
        .map(ApiEntry::signature)
}

pub fn assert_no_collisions() {
    let mut by_signature: HashMap<String, &'static ApiEntry> = HashMap::new();
    for entry in all_entries() {
        let sig = entry.signature();
        if let Some(existing) = by_signature.insert(sig.clone(), entry) {
            panic!(
                "[Reflection] duplicate ApiEntry signature {sig}: {}::{} and {}::{} both declare it",
                existing.owner, existing.rust_fn, entry.owner, entry.rust_fn
            );
        }
    }
    log::debug!(
        "[Reflection] api_table: {} declared APIs, no collisions",
        by_signature.len()
    );
}
