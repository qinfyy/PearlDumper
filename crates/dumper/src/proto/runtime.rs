use std::collections::{HashMap, HashSet};

use anyhow::{Context, ensure};
use il2cpp::{
    get_cached_class, get_native_method,
    vm::{class::Il2CppClass, method::Il2CppMethod},
};
use reflection::runtime_type::RuntimeType;
use serde::Deserialize;
use utils::scanner::game_assembly_function;

use super::{native, type_references::TypeReferences};

#[derive(Deserialize)]
pub(super) struct NativeBinding {
    pub rva: usize,
    prefix: String,
}

#[derive(Deserialize)]
pub(super) struct RegistryBinding {
    class: String,
    field: String,
    #[serde(flatten)]
    native: NativeBinding,
    count: usize,
}

#[derive(Deserialize)]
pub(super) struct XluaBinding {
    class: String,
    fields_class: String,
    delegate_class: String,
    #[serde(flatten)]
    native: NativeBinding,
    count: usize,
}

#[derive(Deserialize)]
pub(super) struct SendBinding {
    pub signature: String,
    #[serde(flatten)]
    native: NativeBinding,
}

#[derive(Deserialize)]
pub(super) struct ProtocolProfile {
    version: String,
    metadata_initializer: NativeBinding,
    object_new: NativeBinding,
    packets: RegistryBinding,
    responses: RegistryBinding,
    xlua: XluaBinding,
    send_methods: Vec<SendBinding>,
}

pub(super) struct PacketRegistry {
    pub all: HashMap<RuntimeType, u16>,
    pub responses: HashMap<RuntimeType, u16>,
    pub handlers: HashMap<RuntimeType, Vec<usize>>,
}

pub(super) struct ProtocolRuntime {
    profile: ProtocolProfile,
    known_methods: HashMap<usize, Il2CppMethod>,
    initialized_usages: HashSet<u32>,
    pub allocation: usize,
}

impl ProtocolRuntime {
    pub fn new() -> anyhow::Result<Self> {
        let profile: ProtocolProfile =
            serde_json::from_str(include_str!("../../../../profiles/4.6.51-protocol.json"))?;
        ensure!(
            profile.version == il2cpp::profile::supported()?.version,
            "protocol profile version mismatch"
        );
        validate_native(&profile.metadata_initializer)?;
        validate_native(&profile.object_new)?;

        // Resolve the public allocator through UnityPlayer's live API table, then check
        // its direct call against the independently identified GameAssembly allocator.
        let api = il2cpp::api::function_address(130)?;
        let api_rva = api
            .checked_sub(*il2cpp::GA_BASE)
            .context("object_new API is outside GameAssembly")?;
        let allocation = *il2cpp::GA_BASE + profile.object_new.rva;
        let calls_allocator = iced_x86::Decoder::with_ip(
            64,
            game_assembly_function(api_rva)?,
            api as u64,
            iced_x86::DecoderOptions::NONE,
        )
        .into_iter()
        .any(|instruction| native::direct_target(&instruction) == Some(allocation));
        ensure!(
            calls_allocator,
            "IL2CPP allocator wrapper does not match the protocol profile"
        );
        ensure!(
            *super::IL2CPP_OBJECT_NEW_RVA.get_or_init(|| profile.object_new.rva)
                == profile.object_new.rva,
            "protocol allocator changed after initialization"
        );

        let methods = il2cpp::FUNCTIONS_TABLE
            .get()
            .context("native method cache is not initialized")?;
        // The signature cache loses unnamed overloads; the ordinal cache retains all
        // MethodInfo pointers, including the empty-name XLua registration methods.
        let known_methods = methods.values().map(|method| (method.0, *method)).collect();
        Ok(Self {
            profile,
            known_methods,
            initialized_usages: HashSet::new(),
            allocation,
        })
    }

    pub fn initialize_references(&mut self, rva: usize, bytes: &[u8]) -> anyhow::Result<()> {
        let helper = *il2cpp::GA_BASE + self.profile.metadata_initializer.rva;
        for usage in native::metadata_usage_lists(bytes, *il2cpp::GA_BASE + rva, helper) {
            if self.initialized_usages.contains(&usage) {
                continue;
            }
            log::debug!("Initializing metadata usage list 0x{usage:X} for RVA 0x{rva:X}");
            // This is the runtime's metadata resolver, not the game method or .cctor.
            // Leave the game's once flag untouched; its normal initialization still owns it.
            microseh::try_seh(|| unsafe {
                let initialize: unsafe extern "C" fn(u32) = std::mem::transmute(helper);
                initialize(usage);
            })
            .map_err(|error| {
                anyhow::anyhow!(
                    "metadata usage list 0x{usage:X} failed at RVA 0x{rva:X}: {error:?}"
                )
            })?;
            self.initialized_usages.insert(usage);
        }
        Ok(())
    }

    pub fn send_targets(&self) -> anyhow::Result<HashMap<usize, native::SendRegisters>> {
        let mut targets = HashMap::new();
        for binding in &self.profile.send_methods {
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
        let packet_rva = validate_registry(&self.profile.packets)?;
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
            packets.len() == self.profile.packets.count,
            "incomplete packet registry: {}/{} entries",
            packets.len(),
            self.profile.packets.count
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

        let response_rva = validate_registry(&self.profile.responses)?;
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
            entries.len() == self.profile.responses.count,
            "incomplete response registry: {}/{} entries",
            entries.len(),
            self.profile.responses.count
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

    pub fn xlua_method_names(&mut self) -> anyhow::Result<HashMap<usize, String>> {
        let binding = &self.profile.xlua;
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
            if let Some(previous) = names.insert(obfuscated.clone(), name.clone()) {
                ensure!(
                    previous == *name,
                    "ambiguous XLua request name for {obfuscated}: {previous} / {name}"
                );
            }
        }

        let mut native_names: HashMap<usize, HashSet<String>> = HashMap::new();
        let methods = il2cpp::FUNCTIONS_TABLE
            .get()
            .context("method cache is missing")?;
        let nested_prefix = format!("{}.", self.profile.xlua.class);
        for (signature, method) in methods {
            let Some((owner, member)) = signature.rsplit_once("::") else {
                continue;
            };
            if owner == self.profile.xlua.class || owner.starts_with(&nested_prefix) {
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
                    .insert(name.clone());
            }
        }
        let result: HashMap<usize, String> = native_names
            .into_iter()
            .filter_map(|(address, names)| {
                if names.len() == 1 {
                    Some((address, names.into_iter().next().unwrap()))
                } else {
                    log::debug!(
                        "Shared request body at RVA 0x{:X} has multiple XLua names: {names:?}",
                        address - *il2cpp::GA_BASE
                    );
                    None
                }
            })
            .collect();
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
    let bytes = game_assembly_function(binding.rva)?;
    let expected = binding
        .prefix
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| Ok(u8::from_str_radix(std::str::from_utf8(pair)?, 16)?))
        .collect::<anyhow::Result<Vec<_>>>()?;
    ensure!(
        !expected.is_empty()
            && binding.prefix.len().is_multiple_of(2)
            && bytes.starts_with(&expected),
        "native code does not match protocol profile at RVA 0x{:X}",
        binding.rva
    );
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
