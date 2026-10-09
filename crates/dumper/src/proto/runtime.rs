use std::collections::{BTreeSet, HashMap, HashSet};

use anyhow::{Context, ensure};
use il2cpp::{
    get_cached_class, get_native_method,
    vm::{class::Il2CppClass, method::Il2CppMethod},
};
use reflection::runtime_type::RuntimeType;
use utils::scanner::game_assembly_function;

use super::{native, type_references::TypeReferences};

pub(super) struct NativeBinding {
    pub rva: usize,
}

pub(super) struct RegistryBinding {
    pub class: String,
    pub field: String,
    pub native: NativeBinding,
    pub count: usize,
}

pub(super) struct XluaBinding {
    pub class: String,
    pub fields_class: String,
    pub delegate_class: String,
    pub native: NativeBinding,
    pub count: usize,
}

pub(super) struct SendBinding {
    pub signature: String,
    pub native: NativeBinding,
}

pub(super) struct ProtocolBindings {
    pub packets: RegistryBinding,
    pub responses: RegistryBinding,
    pub xlua: XluaBinding,
    pub send_methods: Vec<SendBinding>,
}

pub(super) struct PacketRegistry {
    pub all: HashMap<RuntimeType, u16>,
    pub responses: HashMap<RuntimeType, u16>,
    pub handlers: HashMap<RuntimeType, Vec<usize>>,
}

pub(super) struct ProtocolRuntime {
    bindings: ProtocolBindings,
    known_methods: HashMap<usize, Il2CppMethod>,
    initialized_usages: HashSet<u32>,
    pub allocation: usize,
}

impl ProtocolRuntime {
    pub fn new() -> anyhow::Result<Self> {
        let allocation = super::discovery::allocator()?;
        let bindings = super::discovery::discover(allocation)?;
        let allocation_rva = allocation - *il2cpp::GA_BASE;
        ensure!(
            *super::IL2CPP_OBJECT_NEW_RVA.get_or_init(|| allocation_rva) == allocation_rva,
            "IL2CPP allocator changed after discovery"
        );

        let methods = il2cpp::FUNCTIONS_TABLE
            .get()
            .context("native method cache is not initialized")?;
        // The signature cache loses unnamed overloads; the ordinal cache retains all
        // MethodInfo pointers, including the empty-name XLua registration methods.
        let known_methods = methods.values().map(|method| (method.0, *method)).collect();
        Ok(Self {
            bindings,
            known_methods,
            initialized_usages: HashSet::new(),
            allocation,
        })
    }

    pub fn initialize_references(&mut self, rva: usize, bytes: &[u8]) -> anyhow::Result<()> {
        let helper = *il2cpp::GA_BASE + crate::metadata_runtime::plan()?.layout.initializer;
        for usage in native::metadata_usage_lists(bytes, *il2cpp::GA_BASE + rva, helper) {
            if self.initialized_usages.contains(&usage) {
                continue;
            }
            log::debug!("Initializing metadata usage list 0x{usage:X} for RVA 0x{rva:X}");
            // This is the runtime's metadata resolver, not the game method or .cctor.
            // Leave the game's once flag untouched; its normal initialization still owns it.
            crate::metadata_runtime::initialize_list(usage)?;
            self.initialized_usages.insert(usage);
        }
        Ok(())
    }

    pub fn send_targets(&self) -> anyhow::Result<HashMap<usize, native::SendRegisters>> {
        let mut targets = HashMap::new();
        for binding in &self.bindings.send_methods {
            validate_native(&binding.native)?;
            let method = get_native_method(&binding.signature)
                .with_context(|| format!("missing send method {}", binding.signature))?;
            let rva = method.checked_rva()?;
            ensure!(
                rva == binding.native.rva,
                "send method {} moved from 0x{:X} to 0x{rva:X}",
                binding.signature,
                binding.native.rva
            );
            targets.insert(*il2cpp::GA_BASE + rva, native::SendRegisters::STANDARD);
            log::info!(
                "Request send target: {} at RVA 0x{rva:X}",
                binding.signature
            );
        }
        // Open generic definitions legitimately have VA=0. They provide names, not
        // executable addresses; packet coverage comes from the complete registration table.
        Ok(targets)
    }

    pub fn packet_registry(
        &mut self,
        references: &TypeReferences,
    ) -> anyhow::Result<PacketRegistry> {
        let packet_rva = validate_registry(&self.bindings.packets)?;
        let packet_bytes = game_assembly_function(packet_rva)?;
        self.initialize_references(packet_rva, packet_bytes)?;
        let packet_scan = native::scan(
            packet_bytes,
            *il2cpp::GA_BASE + packet_rva,
            self.allocation,
            &HashSet::new(),
            &HashMap::new(),
        )?;
        let packets = packet_scan
            .packets
            .values()
            .max_by_key(|entries| entries.len())
            .context("packet registration calls not found")?;
        ensure!(
            packets.len() == self.bindings.packets.count,
            "incomplete packet registry: {}/{} entries",
            packets.len(),
            self.bindings.packets.count
        );
        let mut all = HashMap::new();
        let mut by_id = HashMap::new();
        for entry in packets {
            let class = references
                .class_from_slot(entry.class_slot)
                .with_context(|| {
                    format!(
                        "unresolved protocol class slot at RVA 0x{:X} for CmdId {}",
                        entry.class_slot - *il2cpp::GA_BASE,
                        entry.id
                    )
                })?;
            ensure!(
                class.byval_arg().0 == class.0 + entry.type_offset,
                "unsupported RuntimeTypeHandle layout for CmdId {}",
                entry.id
            );
            let ty = RuntimeType::from_class(class)?;
            ensure!(
                all.insert(ty, entry.id).is_none(),
                "duplicate protocol type {}",
                ty.il_name()
            );
            ensure!(
                by_id.insert(entry.id, ty).is_none(),
                "duplicate CmdId {}",
                entry.id
            );
        }

        let response_rva = validate_registry(&self.bindings.responses)?;
        let response_bytes = game_assembly_function(response_rva)?;
        self.initialize_references(response_rva, response_bytes)?;
        let response_scan = native::scan(
            response_bytes,
            *il2cpp::GA_BASE + response_rva,
            self.allocation,
            &HashSet::new(),
            &HashMap::new(),
        )?;
        let entries = response_scan
            .handlers
            .values()
            .max_by_key(|entries| entries.len())
            .context("response registration calls not found")?;
        ensure!(
            entries.len() == self.bindings.responses.count,
            "incomplete response registry: {}/{} entries",
            entries.len(),
            self.bindings.responses.count
        );
        let mut responses = HashMap::new();
        let mut handlers: HashMap<RuntimeType, Vec<usize>> = HashMap::new();
        let mut unresolved = 0;
        for entry in entries {
            let &ty = by_id.get(&entry.id).with_context(|| {
                format!("response CmdId {} missing from type registry", entry.id)
            })?;
            ensure!(
                responses.insert(ty, entry.id).is_none(),
                "duplicate response CmdId {}",
                entry.id
            );
            if let Some(method) = self.method_from_slot(entry.method_slot) {
                if let Ok(rva) = method.checked_rva() {
                    handlers.entry(ty).or_default().push(rva);
                }
            } else {
                unresolved += 1;
            }
        }
        if unresolved != 0 {
            log::warn!(
                "{unresolved} response method references are not in the method cache; all packet IDs remain resolved"
            );
        }
        log::info!(
            "Protocol registry: {} packet IDs, {} responses/notifications, {} requests",
            all.len(),
            responses.len(),
            all.len() - responses.len()
        );
        Ok(PacketRegistry {
            all,
            responses,
            handlers,
        })
    }

    fn method_from_slot(&self, address: usize) -> Option<Il2CppMethod> {
        let pointer = unsafe { utils::memory::read::<usize>(address) }.ok()?;
        self.known_methods.get(&pointer).copied()
    }

    pub fn xlua_method_names(&mut self) -> anyhow::Result<HashMap<usize, BTreeSet<String>>> {
        let binding = &self.bindings.xlua;
        let class = get_cached_class(&binding.class).context("XLua translator class is missing")?;
        let fields_class = get_cached_class(&binding.fields_class)
            .context("XLua delegate cache class is missing")?;
        let delegate_class =
            get_cached_class(&binding.delegate_class).context("XLua delegate class is missing")?;
        ensure!(
            il2cpp::api::il2cpp_class_get_declaring_type(fields_class) == class,
            "XLua cache belongs to a different declaring class"
        );
        validate_class_method(class, &binding.native)?;
        let mut fields = HashMap::new();
        for field in fields_class.get_fields() {
            let name = unsafe { utils::cstr_to_str(il2cpp::api::il2cpp_field_get_name(field)) };
            let Some((_, suffix)) = name.split_once("__") else {
                continue;
            };
            ensure!(
                il2cpp::api::il2cpp_field_get_type(field).get_class() == delegate_class,
                "unsupported XLua delegate field {name}"
            );
            fields.insert(
                field.offset(),
                suffix.strip_prefix("Send").unwrap_or(suffix).to_owned(),
            );
        }
        ensure!(
            fields.len() == binding.count,
            "XLua delegate field count mismatch: {}/{}",
            fields.len(),
            binding.count
        );
        let rva = binding.native.rva;
        let bytes = game_assembly_function(rva)?;
        self.initialize_references(rva, bytes)?;
        let scan = native::scan(
            bytes,
            *il2cpp::GA_BASE + rva,
            self.allocation,
            &fields.keys().copied().collect(),
            &HashMap::new(),
        )?;
        ensure!(
            scan.fields.len() == fields.len(),
            "incomplete XLua registration: {}/{} delegate bindings",
            scan.fields.len(),
            fields.len()
        );

        let mut names = HashMap::new();
        for (offset, slot) in scan.fields {
            let method = self.method_from_slot(slot).with_context(|| {
                format!(
                    "XLua method reference at RVA 0x{:X} is not initialized or not cached",
                    slot - *il2cpp::GA_BASE
                )
            })?;
            let name = &fields[&offset];
            if !name.ends_with("Req") {
                continue;
            }
            let obfuscated = method.get_name().into_owned();
            names
                .entry(obfuscated)
                .or_insert_with(BTreeSet::new)
                .insert(name.clone());
        }

        let mut native_names: HashMap<usize, BTreeSet<String>> = HashMap::new();
        let methods = il2cpp::FUNCTIONS_TABLE
            .get()
            .context("method cache is missing")?;
        let nested_prefix = format!("{}.", self.bindings.xlua.class);
        for (signature, method) in methods {
            let Some((owner, member)) = signature.rsplit_once("::") else {
                continue;
            };
            if owner == self.bindings.xlua.class || owner.starts_with(&nested_prefix) {
                continue;
            }
            // Ordinal keys are only a prefilter. Confirm the actual metadata name before
            // accepting a match so numeric suffixes in real method names are not guessed.
            let candidate = member.trim_end_matches(|ch: char| ch.is_ascii_digit());
            let Some(name) = names.get(candidate) else {
                continue;
            };
            if method.get_name() == candidate
                && let Ok(address) = method.checked_va()
            {
                native_names
                    .entry(address)
                    .or_default()
                    .extend(name.iter().cloned());
            }
        }
        let result = native_names;
        log::info!(
            "XLua: {} delegate bindings, {} request names, {} native methods",
            fields.len(),
            names.len(),
            result.len()
        );
        Ok(result)
    }
}

fn validate_native(binding: &NativeBinding) -> anyhow::Result<()> {
    game_assembly_function(binding.rva)?;
    ensure!(
        utils::memory::is_executable(*il2cpp::GA_BASE + binding.rva),
        "non-executable native binding at RVA 0x{:X}",
        binding.rva
    );
    Ok(())
}

fn validate_class_method(class: Il2CppClass, binding: &NativeBinding) -> anyhow::Result<()> {
    validate_native(binding)?;
    let address = *il2cpp::GA_BASE + binding.rva;
    ensure!(
        class
            .get_methods()
            .iter()
            .any(|method| method.va() == address),
        "class {} has no native method at RVA 0x{:X}",
        class.byval_arg().il_name(),
        binding.rva
    );
    Ok(())
}

fn validate_registry(binding: &RegistryBinding) -> anyhow::Result<usize> {
    let class = get_cached_class(&binding.class)
        .with_context(|| format!("missing packet registry class {}", binding.class))?;
    validate_class_method(class, &binding.native)?;
    ensure!(
        class.get_fields().iter().any(|&field| unsafe {
            utils::cstr_to_str(il2cpp::api::il2cpp_field_get_name(field))
        } == binding.field),
        "missing registry field {}::{}",
        binding.class,
        binding.field
    );
    Ok(binding.native.rva)
}
