use std::collections::{BTreeSet, HashMap, HashSet};

use anyhow::{Context, Result, ensure};
use iced_x86::{Decoder, DecoderOptions, Mnemonic};
use il2cpp::{
    api,
    vm::{class::Il2CppClass, method::Il2CppMethod},
};
use utils::scanner::{game_assembly_candidate, game_assembly_function};

use super::{
    native,
    runtime::{NativeBinding, ProtocolBindings, RegistryBinding, SendBinding, XluaBinding},
};

fn calls(rva: usize) -> Result<Vec<usize>> {
    Ok(Decoder::with_ip(
        64,
        game_assembly_function(rva)?,
        (*il2cpp::GA_BASE + rva) as u64,
        DecoderOptions::NONE,
    )
    .into_iter()
    .filter_map(|i| native::direct_target(&i))
    .collect())
}

pub(super) fn allocator() -> Result<usize> {
    let api = api::function_address(130)?;
    let rva = api
        .checked_sub(*il2cpp::GA_BASE)
        .context("allocator wrapper outside GameAssembly")?;
    let targets: BTreeSet<_> = calls(rva)?
        .into_iter()
        .filter(|target| *target != api)
        .collect();
    ensure!(
        targets.len() == 1,
        "allocator wrapper has ambiguous targets: {targets:X?}"
    );
    let target = *targets.first().unwrap();
    ensure!(
        utils::memory::is_executable(target),
        "allocator target is not executable"
    );
    game_assembly_function(
        target
            .checked_sub(*il2cpp::GA_BASE)
            .context("allocator outside GameAssembly")?,
    )?;
    log::info!("Dynamic allocator: RVA 0x{:X}", target - *il2cpp::GA_BASE);
    Ok(target)
}

fn scan(
    bytes: &[u8],
    rva: usize,
    allocation: usize,
    offsets: &HashSet<usize>,
) -> Result<native::Registrations> {
    native::scan(
        bytes,
        *il2cpp::GA_BASE + rva,
        allocation,
        offsets,
        &HashMap::new(),
    )
}

fn candidate_bytes(method: Il2CppMethod) -> Result<Option<(usize, &'static [u8])>> {
    if method.va() == 0 {
        log::debug!(
            "Skipping protocol candidate {}: no native method pointer",
            signature(method)
        );
        return Ok(None);
    }
    let rva = method
        .checked_rva()
        .with_context(|| format!("reading protocol candidate {}", signature(method)))?;
    let bytes = game_assembly_candidate(rva).with_context(|| {
        format!(
            "reading protocol candidate {} at RVA 0x{rva:X}",
            signature(method)
        )
    })?;
    if bytes.is_none() {
        log::debug!(
            "Skipping protocol candidate {} at RVA 0x{rva:X}: no PE function boundary record",
            signature(method)
        );
    }
    Ok(bytes.map(|bytes| (rva, bytes)))
}

fn signature(method: Il2CppMethod) -> String {
    let parameters = (0..api::il2cpp_method_get_param_count(method))
        .map(|index| {
            api::il2cpp_method_get_param(method, index)
                .il_name()
                .into_owned()
        })
        .collect::<Vec<_>>();
    format!(
        "{}::{}({})",
        method.class().byval_arg().il_name(),
        method.get_name(),
        parameters.join(",")
    )
}

fn param_types(method: Il2CppMethod, expected: &[&str]) -> bool {
    api::il2cpp_method_get_param_count(method) as usize == expected.len()
        && expected.iter().enumerate().all(|(index, name)| {
            api::il2cpp_method_get_param(method, index as u32).il_name() == *name
        })
}

fn complete_group<T>(
    rva: usize,
    groups: &std::collections::BTreeMap<usize, Vec<T>>,
) -> Result<usize> {
    ensure!(
        groups.len() == 1,
        "registry 0x{rva:X} has {} insertion targets",
        groups.len()
    );
    let (&target, entries) = groups.first_key_value().unwrap();
    let direct = Decoder::with_ip(
        64,
        game_assembly_function(rva)?,
        (*il2cpp::GA_BASE + rva) as u64,
        DecoderOptions::NONE,
    )
    .into_iter()
    .filter(|i| i.mnemonic() == Mnemonic::Call && native::direct_target(i) == Some(target))
    .count();
    ensure!(
        !entries.is_empty() && direct == entries.len(),
        "registry 0x{rva:X}: only {}/{} insert calls decoded",
        entries.len(),
        direct
    );
    Ok(entries.len())
}

pub(super) fn discover(allocation: usize) -> Result<ProtocolBindings> {
    let classes = il2cpp::CLASS_TABLE_VEC
        .get()
        .context("class cache missing")?;
    let mut packets = Vec::new();
    let mut responses = Vec::new();
    let mut xlua = Vec::new();
    for &class in classes {
        let fields = class.get_fields();
        let mut packet_fields = Vec::new();
        let mut response_fields = Vec::new();
        let mut send_fields = Vec::new();
        for field in fields {
            let ty = api::il2cpp_field_get_type(field);
            let name = unsafe { utils::cstr_to_str(api::il2cpp_field_get_name(field)) };
            let type_name = ty.il_name().replace(' ', "");
            if type_name.contains("Dictionary")
                && type_name.contains("<System.RuntimeTypeHandle,System.UInt16>")
            {
                packet_fields.push(name.to_string());
            }
            if type_name.contains("Dictionary") && type_name.contains("<System.UInt16,") {
                response_fields.push(name.to_string());
            }
            if name
                .split_once("__")
                .is_some_and(|(_, suffix)| suffix.starts_with("Send") && suffix.ends_with("Req"))
            {
                send_fields.push((field.offset(), ty.get_class()));
            }
        }
        if !packet_fields.is_empty() || !response_fields.is_empty() {
            for method in class
                .get_methods()
                .into_iter()
                .filter(|method| method.get_name() == ".cctor")
            {
                let Some((rva, bytes)) = candidate_bytes(method)? else {
                    continue;
                };
                let registry =
                    scan(bytes, rva, allocation, &HashSet::new()).with_context(|| {
                        format!(
                            "scanning registry candidate {} at RVA 0x{rva:X}",
                            signature(method)
                        )
                    })?;
                if !packet_fields.is_empty() && !registry.packets.is_empty() {
                    ensure!(
                        packet_fields.len() == 1,
                        "ambiguous packet dictionary in {}",
                        class.byval_arg().il_name()
                    );
                    packets.push(RegistryBinding {
                        class: class.byval_arg().il_name().into_owned(),
                        field: packet_fields[0].clone(),
                        native: NativeBinding { rva },
                        count: complete_group(rva, &registry.packets)?,
                    });
                }
                if !response_fields.is_empty() && !registry.handlers.is_empty() {
                    ensure!(
                        response_fields.len() == 1,
                        "ambiguous response dictionary in {}",
                        class.byval_arg().il_name()
                    );
                    responses.push(RegistryBinding {
                        class: class.byval_arg().il_name().into_owned(),
                        field: response_fields[0].clone(),
                        native: NativeBinding { rva },
                        count: complete_group(rva, &registry.handlers)?,
                    });
                }
            }
        }
        if !send_fields.is_empty() {
            let owner = api::il2cpp_class_get_declaring_type(class);
            if owner.0 == 0 {
                continue;
            }
            let delegate = send_fields[0].1;
            if send_fields.iter().any(|(_, ty)| *ty != delegate) {
                continue;
            }
            let parent = api::il2cpp_class_get_parent(delegate);
            if parent.0 == 0 || parent.byval_arg().il_name() != "System.MulticastDelegate" {
                continue;
            }
            // Include every cached delegate, not just Send-prefixed field names.
            let offsets: HashSet<_> = class
                .get_fields()
                .into_iter()
                .filter(|field| api::il2cpp_field_get_type(*field).get_class() == delegate)
                .map(|field| field.offset())
                .collect();
            for method in owner.get_methods() {
                let Some((rva, bytes)) = candidate_bytes(method)? else {
                    continue;
                };
                if bytes.len() < offsets.len() * 5 {
                    continue;
                }
                let registration = scan(bytes, rva, allocation, &offsets).with_context(|| {
                    format!(
                        "scanning XLua candidate {} at RVA 0x{rva:X}",
                        signature(method)
                    )
                })?;
                if registration.fields.len() == offsets.len() {
                    xlua.push(XluaBinding {
                        class: owner.byval_arg().il_name().into_owned(),
                        fields_class: class.byval_arg().il_name().into_owned(),
                        delegate_class: delegate.byval_arg().il_name().into_owned(),
                        native: NativeBinding { rva },
                        count: offsets.len(),
                    });
                }
            }
        }
    }
    ensure!(
        packets.len() == 1 && responses.len() == 1 && xlua.len() == 1,
        "dynamic registries are not unique: packets={}, responses={}, XLua={}",
        packets.len(),
        responses.len(),
        xlua.len()
    );
    let send_methods = discover_sends(classes)?;
    let bindings = ProtocolBindings {
        packets: packets.remove(0),
        responses: responses.remove(0),
        xlua: xlua.remove(0),
        send_methods,
    };
    log::info!(
        "Dynamic protocol: packets {} @0x{:X} ({}), responses {} @0x{:X} ({}), XLua {} @0x{:X} ({})",
        bindings.packets.class,
        bindings.packets.native.rva,
        bindings.packets.count,
        bindings.responses.class,
        bindings.responses.native.rva,
        bindings.responses.count,
        bindings.xlua.class,
        bindings.xlua.native.rva,
        bindings.xlua.count
    );
    Ok(bindings)
}

fn discover_sends(classes: &[Il2CppClass]) -> Result<Vec<SendBinding>> {
    let manager =
        il2cpp::get_cached_class("RPG.Client.NetworkManager").context("NetworkManager missing")?;
    let roots = manager
        .get_methods()
        .into_iter()
        .filter(|method| {
            param_types(
                *method,
                &[
                    "System.UInt16",
                    "Google.Protobuf.IMessage",
                    "System.Boolean",
                ],
            ) && method.va() != 0
        })
        .collect::<Vec<_>>();
    ensure!(
        roots.len() == 1,
        "ambiguous network send signature: {} candidates",
        roots.len()
    );
    let root = roots[0];
    let mut selected = HashSet::from([root.checked_va()?]);
    let mut methods = vec![root];
    let mut candidates = Vec::new();
    for class in classes {
        for method in class.get_methods() {
            if param_types(method, &["System.UInt16", "Google.Protobuf.IMessage"])
                && method.va() != 0
            {
                let rva = method.checked_rva()?;
                // Packet builders consume the same (CmdId, IMessage) pair without
                // calling NetworkManager. Their typed result is an independent root.
                let return_type = api::il2cpp_method_get_return_type(method);
                if return_type.0 != 0
                    && return_type.il_name() == "RPG.Client.NetPacket"
                    && selected.insert(method.checked_va()?)
                {
                    methods.push(method);
                }
                candidates.push((method, calls(rva)?));
            }
        }
    }
    loop {
        let before = selected.len();
        for (method, callees) in &candidates {
            if !selected.contains(&method.va())
                && callees.iter().any(|callee| selected.contains(callee))
            {
                selected.insert(method.va());
                methods.push(*method);
            }
        }
        if before == selected.len() {
            break;
        }
    }
    methods
        .into_iter()
        .map(|method| {
            Ok(SendBinding {
                signature: signature(method),
                native: NativeBinding {
                    rva: method.checked_rva()?,
                },
            })
        })
        .collect()
}
