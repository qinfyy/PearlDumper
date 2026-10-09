use std::{collections::HashMap, sync::OnceLock};

use crate::vm::method::Il2CppMethod;

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

pub fn resolved_method(signature: &str) -> Option<Il2CppMethod> {
    RESOLVED_APIS.get()?.get(signature).copied()
}

fn overload_parameters(entry: &ApiEntry) -> Option<&'static [&'static str]> {
    match (entry.class, entry.member) {
        ("System.Attribute", "GetCustomAttribute") => {
            Some(&["System.Reflection.MemberInfo", "System.Type"])
        }
        ("System.Type", "GetMethod" | "GetProperty") => {
            Some(&["System.String", "System.Reflection.BindingFlags"])
        }
        ("System.Type", "GetType") => Some(&["System.String", "System.Boolean", "System.Boolean"]),
        ("System.RuntimeType", "IsAssignableFrom") => Some(&["System.Type"]),
        _ => None,
    }
}

pub fn bind_dynamic() -> anyhow::Result<()> {
    if RESOLVED_APIS.get().is_some() {
        return Ok(());
    }
    let mut resolved = HashMap::new();
    for entry in all_entries() {
        let signature = entry.signature();
        let class = crate::get_cached_class(entry.class)
            .ok_or_else(|| anyhow::anyhow!("missing reflection class {}", entry.class))?;
        let matches = class
            .get_methods()
            .into_iter()
            .filter(|method| {
                method.get_name() == entry.member
                    && crate::api::il2cpp_method_get_param_count(*method) as usize
                        == entry.parameter_count
                    && overload_parameters(entry).is_none_or(|parameters| {
                        parameters.iter().enumerate().all(|(index, expected)| {
                            crate::api::il2cpp_method_get_param(*method, index as u32).il_name()
                                == *expected
                        })
                    })
            })
            .collect::<Vec<_>>();
        anyhow::ensure!(
            matches.len() == 1,
            "{signature}: dynamic signature matched {} methods; refusing to use an ordinal or guessed RVA",
            matches.len()
        );
        let method = matches[0];
        method.checked_va()?;
        log::debug!(
            "Dynamic reflection binding {signature}: RVA 0x{:X}",
            method.rva()
        );
        anyhow::ensure!(
            resolved.insert(signature.clone(), method).is_none(),
            "duplicate reflection declaration {signature}"
        );
    }
    log::info!(
        "Resolved {} reflection bindings from live method signatures",
        resolved.len()
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
