mod types;

use std::{
    collections::{HashMap, HashSet},
    fmt::Write as FmtWrite,
    fs::{self, File},
    io::{BufWriter, Write as IoWrite},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use il2cpp::{
    api,
    vm::{class::Il2CppClass, method::Il2CppMethod},
};
use reflection::attributes::{FieldAttributes, MethodAttributes, TypeAttributes};

use self::types::Types;

#[derive(Default)]
struct Stats {
    types: usize,
    empty_type_names: usize,
    fields: usize,
    methods: usize,
    constructors: usize,
    fields_time: Duration,
    methods_time: Duration,
}

pub fn dump() -> Result<()> {
    let started = Instant::now();
    let classes = il2cpp::CLASS_TABLE_VEC
        .get()
        .context("type cache is not initialized")?;
    let total = classes.len();
    let mut names = Types::new()?;
    let assembly_base = *il2cpp::GA_BASE;
    let assembly_size = il2cpp::profile::current()?.assembly.image_size as usize;
    let partial = "DUMP/dump.cs.partial";
    let final_name = "DUMP/dump.cs";
    let file = File::create(partial).with_context(|| format!("creating {partial}"))?;
    let mut out = BufWriter::with_capacity(1024 * 1024, file);
    writeln!(out, "// CSharp rendered")?;
    let mut assemblies = dump_header(&mut out, classes)?;
    let mut stats = Stats::default();
    let mut seen = HashSet::with_capacity(total);
    let mut body = String::with_capacity(16 * 1024);
    let mut last_progress = started;
    let mut last_index = 0;
    log::info!(
        "Csharp renderer: {total} type definitions; types/fields/methods only, no managed reflection"
    );

    for (index, &class) in classes.iter().enumerate() {
        if index == 0 || last_progress.elapsed() >= Duration::from_secs(2) {
            let speed =
                (index - last_index) as f64 / last_progress.elapsed().as_secs_f64().max(0.001);
            let remaining = if speed > 0.0 {
                (total - index) as f64 / speed
            } else {
                0.0
            };
            log::info!(
                "Csharp native export: {index}/{total}, types={}, fields={}, methods={}, constructors={}, {:.0} types/s, {:.1}s elapsed, ETA {:.1}s, next class=0x{:X}",
                stats.types,
                stats.fields,
                stats.methods,
                stats.constructors,
                speed,
                started.elapsed().as_secs_f64(),
                remaining,
                class.0
            );
            last_progress = Instant::now();
            last_index = index;
        }
        ensure!(class.0 != 0, "null class for typedef index {index}");
        if !seen.insert(class.0) {
            continue;
        }
        let type_started = Instant::now();
        let fields_before = stats.fields_time;
        let methods_before = stats.methods_time;
        let image = class.get_image();
        ensure!(
            image.0 != 0,
            "typedef {index}, class=0x{:X}: null image",
            class.0
        );
        let assembly = assemblies
            .entry(image.0)
            .or_insert_with(|| image.get_name().into_owned());
        let namespace = class.get_namespace();
        let type_name = class.get_name();
        let type_name = if type_name.is_empty() {
            stats.empty_type_names += 1;
            log::debug!(
                "Csharp empty class name: {assembly}: typedef {index}, class=0x{:X}; writing \"\" and preserving members",
                class.0
            );
            "\"\""
        } else {
            type_name.as_ref()
        };
        body.clear();
        writeln!(body, "// TypeDefIndex: {index}")?;
        writeln!(body, "// Assembly: {assembly}")?;
        writeln!(body, "// Namespace: {namespace}")?;
        render_type(
            class,
            type_name,
            &mut names,
            &mut stats,
            &mut body,
            assembly_base,
            assembly_size,
        )
        .with_context(|| {
            format!(
                "{assembly}: typedef {index} {namespace}.{type_name}, class=0x{:X}",
                class.0
            )
        })?;
        out.write_all(body.as_bytes())?;
        if type_started.elapsed() >= Duration::from_secs(1) {
            log::warn!(
                "Csharp slow type: {assembly}: {namespace}.{type_name}, class=0x{:X}, {:.2}s, fields {:.2}s / methods {:.2}s",
                class.0,
                type_started.elapsed().as_secs_f64(),
                (stats.fields_time - fields_before).as_secs_f64(),
                (stats.methods_time - methods_before).as_secs_f64()
            );
        }
    }
    out.flush()?;
    out.get_ref().sync_all()?;
    drop(out);
    fs::rename(partial, final_name).with_context(|| format!("publishing {final_name}"))?;
    log::info!(
        "Csharp renderer complete: types={}, empty type names={}, fields={}, constructors={}, methods={}, cached names={}, memory queries={}, fields {:.2}s, methods {:.2}s, total {:.2}s",
        stats.types,
        stats.empty_type_names,
        stats.fields,
        stats.constructors,
        stats.methods,
        names.cached_names(),
        names.memory.query_count(),
        stats.fields_time.as_secs_f64(),
        stats.methods_time.as_secs_f64(),
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

fn dump_header(out: &mut impl IoWrite, classes: &[Il2CppClass]) -> Result<HashMap<usize, String>> {
    let domain = api::il2cpp_domain_get();
    ensure!(domain.0 != 0, "Csharp header: null IL2CPP domain");
    let mut assembly_count = 0;
    let assembly_array = api::il2cpp_domain_get_assemblies(domain, &mut assembly_count);
    ensure!(
        (1..=1024).contains(&assembly_count)
            && utils::memory::is_readable(
                assembly_array as usize,
                assembly_count * size_of::<usize>()
            ),
        "Csharp header: invalid assembly list (count {assembly_count})"
    );
    let mut names = HashMap::with_capacity(assembly_count);
    let mut running_offset = 0usize;
    for index in 0..assembly_count {
        let assembly = unsafe { *assembly_array.add(index) };
        if assembly.0 == 0 {
            continue;
        }
        let image = api::il2cpp_assembly_get_image(assembly);
        if image.0 == 0 {
            continue;
        }
        let name = image.get_name().into_owned();
        let class_count = api::il2cpp_image_get_class_count(image);
        let next_offset = running_offset
            .checked_add(class_count)
            .context("Csharp header: type index overflow")?;
        let image_classes = classes.get(running_offset..next_offset).with_context(|| {
            format!("Csharp image {index} {name}: type range exceeds initialized cache")
        })?;
        // Header offsets must describe the same assembly order as the cached TypeDefIndex values.
        if let (Some(first), Some(last)) = (image_classes.first(), image_classes.last()) {
            ensure!(
                first.0 != 0
                    && last.0 != 0
                    && first.get_image().0 == image.0
                    && last.get_image().0 == image.0,
                "Csharp image {index} {name}: type range does not match initialized cache"
            );
        }
        writeln!(
            out,
            "// Image {index}: {name} - TypeDefIndexOffset: {running_offset}"
        )?;
        names.insert(image.0, name);
        running_offset = next_offset;
    }
    ensure!(
        running_offset == classes.len(),
        "Csharp header: {running_offset} image types do not match {} cached types",
        classes.len()
    );
    writeln!(out)?;
    Ok(names)
}

fn render_type(
    class: Il2CppClass,
    type_name: &str,
    names: &mut Types,
    stats: &mut Stats,
    out: &mut String,
    assembly_base: usize,
    assembly_size: usize,
) -> Result<()> {
    let attrs = TypeAttributes::from_bits_retain(class.get_flags());
    let is_enum = class.is_enum();
    let is_value = class.is_valuetype();
    if attrs.contains(TypeAttributes::Serializable) {
        out.push_str("[Serializable]\n");
    }
    type_modifier(attrs, is_enum, is_value, out);
    out.push_str(type_name);
    let mut has_parent = false;
    let base = class.get_parent();
    if !is_value && !is_enum && base.0 != 0 {
        let name = names.name(base.byval_arg())?;
        if name != "object" {
            write!(out, " : {name}")?;
            has_parent = true;
        }
    }
    for interface in class.get_interfaces() {
        ensure!(interface.0 != 0, "null interface");
        write!(
            out,
            "{}{}",
            if has_parent { ", " } else { " : " },
            names.name(interface.byval_arg())?
        )?;
        has_parent = true;
    }
    out.push_str("\n{\n\t// Fields\n");
    let fields_started = Instant::now();
    let mut iter = std::ptr::null();
    let mut count = 0;
    loop {
        let field = api::il2cpp_class_get_fields(class, &mut iter);
        if field.0 == 0 {
            break;
        }
        count += 1;
        ensure!(count <= 65536, "field iteration did not terminate");
        let field_name = field.get_name();
        let attrs = FieldAttributes::from_bits_retain(field.get_flags());
        let mut offset = field.offset();
        if is_value
            && !attrs.intersects(FieldAttributes::Static | FieldAttributes::Literal)
            && offset > 0
        {
            offset = offset
                .checked_sub(16)
                .context("value-type field offset precedes object header")?;
        }
        out.push('\t');
        field_modifier(attrs, out);
        let ty = names.name(field.get_type()).with_context(|| {
            format!(
                "field {field_name}, handle=0x{:X}, offset=0x{offset:X}",
                field.0
            )
        })?;
        writeln!(out, "{ty} {field_name}; // Offset: 0x{offset:X}")?;
        stats.fields += 1;
    }
    stats.fields_time += fields_started.elapsed();

    out.push_str("\n\t// Methods\n");
    let methods_started = Instant::now();
    let mut iter = std::ptr::null();
    let mut count = 0;
    loop {
        let method = api::il2cpp_class_get_methods(class, &mut iter);
        if method.0 == 0 {
            break;
        }
        count += 1;
        ensure!(count <= 65536, "method iteration did not terminate");
        let method_name = method.get_name();
        let va = method
            .try_va_cached(&mut names.memory)
            .with_context(|| format!("method {method_name}, handle=0x{:X}: address", method.0))?;
        render_method(
            method,
            method_name.as_ref(),
            type_name,
            va,
            names,
            out,
            assembly_base,
            assembly_size,
        )
        .with_context(|| {
            format!(
                "method {method_name}, handle=0x{:X}, VA=0x{va:X}, RVA={:?}",
                method.0,
                va.checked_sub(assembly_base)
            )
        })?;
        if method_name == ".ctor" || method_name == ".cctor" {
            stats.constructors += 1;
        } else {
            stats.methods += 1;
        }
    }
    stats.methods_time += methods_started.elapsed();
    out.push_str("}\n\n");
    stats.types += 1;
    Ok(())
}

fn type_modifier(attrs: TypeAttributes, is_enum: bool, is_value: bool, out: &mut String) {
    out.push_str(match attrs & TypeAttributes::VisibilityMask {
        TypeAttributes::Public | TypeAttributes::NestedPublic => "public ",
        TypeAttributes::NestedPrivate => "private ",
        TypeAttributes::NestedFamily => "protected ",
        TypeAttributes::NestedFamORAssem => "protected internal ",
        _ => "internal ",
    });
    let is_interface = attrs.contains(TypeAttributes::Interface);
    if attrs.contains(TypeAttributes::Abstract) && attrs.contains(TypeAttributes::Sealed) {
        out.push_str("static ");
    } else if attrs.contains(TypeAttributes::Abstract) && !is_interface {
        out.push_str("abstract ");
    } else if attrs.contains(TypeAttributes::Sealed) && !is_enum && !is_value {
        out.push_str("sealed ");
    }
    out.push_str(if is_interface {
        "interface "
    } else if is_enum {
        "enum "
    } else if is_value {
        "struct "
    } else {
        "class "
    });
}

fn method_modifier(attrs: MethodAttributes, out: &mut String) {
    out.push_str(match attrs & MethodAttributes::MemberAccessMask {
        MethodAttributes::Private => "private ",
        MethodAttributes::Family => "protected ",
        MethodAttributes::Assembly | MethodAttributes::FamANDAssem => "internal ",
        MethodAttributes::FamORAssem => "protected internal ",
        MethodAttributes::Public => "public ",
        _ => "",
    });
    if attrs.contains(MethodAttributes::Static) {
        out.push_str("static ");
    }
    if attrs.contains(MethodAttributes::Abstract) {
        out.push_str("abstract ");
    } else if attrs.contains(MethodAttributes::Final)
        && attrs.contains(MethodAttributes::Virtual)
        && !attrs.contains(MethodAttributes::NewSlot)
    {
        out.push_str("sealed override ");
    } else if attrs.contains(MethodAttributes::Virtual) {
        out.push_str(if attrs.contains(MethodAttributes::NewSlot) {
            "virtual "
        } else {
            "override "
        });
    }
    if attrs.contains(MethodAttributes::PinvokeImpl) {
        out.push_str("extern ");
    }
}

fn field_modifier(attrs: FieldAttributes, out: &mut String) {
    out.push_str(match attrs & FieldAttributes::FieldAccessMask {
        FieldAttributes::Private => "private ",
        FieldAttributes::Family => "protected ",
        FieldAttributes::Assembly | FieldAttributes::FamANDAssem => "internal ",
        FieldAttributes::FamORAssem => "protected internal ",
        FieldAttributes::Public => "public ",
        _ => "",
    });
    if attrs.contains(FieldAttributes::Literal) {
        out.push_str("const ");
    } else {
        if attrs.contains(FieldAttributes::Static) {
            out.push_str("static ");
        }
        if attrs.contains(FieldAttributes::InitOnly) {
            out.push_str("readonly ");
        }
    }
}

fn render_method(
    method: Il2CppMethod,
    name: &str,
    type_name: &str,
    va: usize,
    names: &mut Types,
    out: &mut String,
    assembly_base: usize,
    assembly_size: usize,
) -> Result<()> {
    if va != 0 {
        let rva = va
            .checked_sub(assembly_base)
            .context("method precedes GameAssembly")?;
        ensure!(
            rva < assembly_size && names.memory.is_executable(va),
            "method address outside executable GameAssembly"
        );
        writeln!(out, "\t// RVA: 0x{rva:X} VA: 0x{va:X}")?;
    } else {
        out.push_str("\t// no native address\n");
    }
    let flags_address = method
        .0
        .checked_add(names.method_flags_offset)
        .context("method flags address overflow")?;
    let flags = unsafe { names.memory.read::<u16>(flags_address) }?;
    let attrs = MethodAttributes::from_bits_retain(i32::from(flags));
    ensure!(
        api::il2cpp_method_is_instance(method) != attrs.contains(MethodAttributes::Static),
        "method flags disagree with native instance API"
    );
    out.push('\t');
    method_modifier(attrs, out);
    let is_ctor = name == ".ctor" || name == ".cctor";
    if is_ctor {
        // Native metadata names, including generic arity, follow the reference dumper.
        out.push_str(type_name);
    } else {
        let ty = method.get_return_type()?;
        if api::il2cpp_type_is_byref(ty) {
            out.push_str("ref ");
        }
        write!(out, "{} {name}", names.name(ty)?.trim_end_matches('&'))?;
    }
    out.push('(');
    let count = api::il2cpp_method_get_param_count(method);
    ensure!(count <= 65536, "invalid parameter count {count}");
    for index in 0..count {
        if index != 0 {
            out.push_str(", ");
        }
        let ty = api::il2cpp_method_get_param(method, index);
        ensure!(ty.0 != 0, "parameter {index} has null type");
        let attrs = api::il2cpp_type_get_attrs(ty);
        if api::il2cpp_type_is_byref(ty) {
            out.push_str(match attrs & 3 {
                2 => "out ",
                1 => "in ",
                _ => "ref ",
            });
        } else {
            if attrs & 1 != 0 {
                out.push_str("[In] ");
            }
            if attrs & 2 != 0 {
                out.push_str("[Out] ");
            }
        }
        write!(out, "{} p{index}", names.name(ty)?.trim_end_matches('&'))?;
    }
    out.push_str(") { }\n");
    Ok(())
}
