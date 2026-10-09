use std::collections::HashMap;

use anyhow::{Context, Result, ensure};
use il2cpp::{
    api,
    vm::{method::method_flags_offset_from_instance, r#type::Il2CppType, value::Il2CppValue},
};
use utils::memory::RegionCache;

pub(super) struct Types {
    pub memory: RegionCache,
    pub method_flags_offset: usize,
    names: HashMap<usize, String>,
}

impl Types {
    pub fn new() -> Result<Self> {
        let instance = api::function_address(122)?;
        let rva = instance
            .checked_sub(*il2cpp::GA_BASE)
            .context("method instance API is outside GameAssembly")?;
        let image_size = il2cpp::profile::current()?.assembly.image_size as usize;
        ensure!(
            rva.checked_add(11).is_some_and(|end| end <= image_size),
            "method instance API exceeds GameAssembly"
        );
        // This bounded instruction template includes RET; it is not a guessed function range.
        let code = unsafe { utils::memory::read_bytes(instance, 11) }?;
        let method_flags_offset = method_flags_offset_from_instance(&code)?;
        log::info!(
            "Csharp native layout: method flags +0x{method_flags_offset:X}, instance API RVA 0x{rva:X}"
        );
        Ok(Self {
            memory: RegionCache::default(),
            method_flags_offset,
            names: HashMap::new(),
        })
    }

    pub fn name(&mut self, ty: Il2CppType) -> Result<&str> {
        ensure!(!ty.is_null(), "null native signature type");
        if let std::collections::hash_map::Entry::Vacant(entry) = self.names.entry(ty.0) {
            let raw = ty.full_name();
            if raw.is_empty() {
                log::warn!(
                    "Csharp empty signature type name: type=0x{:X}; writing \"\"",
                    ty.0
                );
            }
            let mut name = String::with_capacity(raw.len());
            if raw.is_empty() {
                name.push_str("\"\"");
            }
            // Replace whole identifiers, including primitives within generic arguments.
            let mut start = 0;
            for (offset, ch) in raw.char_indices() {
                if matches!(ch, '<' | '>' | ',' | '[' | ']' | '&' | '*' | ' ') {
                    name.push_str(alias(&raw[start..offset]));
                    name.push(ch);
                    start = offset + ch.len_utf8();
                }
            }
            name.push_str(alias(&raw[start..]));
            entry.insert(name);
        }
        self.names
            .get(&ty.0)
            .map(String::as_str)
            .context("native name cache entry missing")
    }

    pub fn cached_names(&self) -> usize {
        self.names.len()
    }
}

fn alias(name: &str) -> &str {
    match name {
        "System.Void" => "void",
        "System.Boolean" => "bool",
        "System.Byte" => "byte",
        "System.SByte" => "sbyte",
        "System.Int16" => "short",
        "System.UInt16" => "ushort",
        "System.Int32" => "int",
        "System.UInt32" => "uint",
        "System.Int64" => "long",
        "System.UInt64" => "ulong",
        "System.Single" => "float",
        "System.Double" => "double",
        "System.Char" => "char",
        "System.String" => "string",
        "System.Object" => "object",
        "System.Decimal" => "decimal",
        _ => name,
    }
}
