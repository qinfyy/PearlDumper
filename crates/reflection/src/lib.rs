use std::{
    panic::catch_unwind,
    time::{Duration, Instant},
};

use il2cpp::vm::metadata_cache;
use indexmap::IndexMap;

use crate::{method_info::MethodInfo, runtime_type::RuntimeType};
pub mod array;
pub mod assembly;
pub mod attributes;
pub mod r#enum;
pub mod event_info;
pub mod field_info;
pub mod member_info;
pub mod method_info;
pub mod parameter_info;
pub mod property_info;
pub mod runtime_type;
pub mod self_test;

pub mod custom_attribute;

pub mod serializer;

pub fn init_reflection_methods() -> anyhow::Result<()> {
    let started = Instant::now();
    log::info!("Initializing reflection bindings");
    il2cpp::api_table::assert_no_collisions();
    il2cpp::api_table::bind_dynamic()?;
    test_reflection_apis()?;
    let mut out = IndexMap::new();
    let type_count = unsafe { il2cpp::MAX_TYPEDEFINDEX };
    let mut last_progress = Instant::now();
    log::info!("Building reflection method signatures for {type_count} types");
    for i in 0..type_count {
        if i == 0 || last_progress.elapsed() >= Duration::from_secs(2) {
            log::info!(
                "Reflection scan: {i}/{type_count} types, {} methods, {:.1}s elapsed",
                out.len(),
                started.elapsed().as_secs_f64()
            );
            last_progress = Instant::now();
        }
        let class = metadata_cache::get_typeinfo_from_typedefindex(i);
        let rt = RuntimeType::from_class(class).unwrap();
        for method in class.get_methods() {
            let method_info = MethodInfo::from_handle(method).unwrap();
            out.insert(
                format!("{}::{}", rt.il_name(), method_info.signature()),
                method,
            );
        }
    }
    log::info!("Reflection scan complete: {} methods", out.len());
    il2cpp::FUNCTIONS_TABLE_REFLECTION.set(out).unwrap();
    dump_methods_reflection_json()?;
    log::info!(
        "Reflection initialization complete in {:.2}s",
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

#[inline]
pub fn test_reflection_apis() -> anyhow::Result<()> {
    let tests = self_test::all_tests();
    let started = Instant::now();
    log::info!("Starting {} runtime reflection checks", tests.len());
    for (index, case) in tests.iter().enumerate() {
        let check_started = Instant::now();
        log::info!(
            "Running reflection check {}/{}: {}",
            index + 1,
            tests.len(),
            case.display_id()
        );
        if let Some(error) = run_single_test(case) {
            anyhow::bail!(
                "reflection test {} failed: {error}; no alternative methods were called",
                case.display_id()
            );
        }
        log::debug!(
            "Passed reflection check {} in {:.3}s",
            case.display_id(),
            check_started.elapsed().as_secs_f64()
        );
    }
    log::info!(
        "Passed {} runtime reflection checks in {:.2}s",
        tests.len(),
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

fn run_single_test(case: &self_test::TestCase) -> Option<String> {
    let result = microseh::try_seh(|| catch_unwind(std::panic::AssertUnwindSafe(case.run)));

    match result {
        Err(seh_err) => Some(format!(
            "SEH exception in {}: {seh_err:?}",
            case.display_id()
        )),
        Ok(Err(panic)) => Some(format!("panic in {}: {panic:?}", case.display_id())),
        Ok(Ok(Err(err))) => Some(format!("{err:?}")),
        Ok(Ok(Ok(()))) => None,
    }
}

pub fn dump_methods_reflection_json() -> anyhow::Result<()> {
    il2cpp::method_dump::write(
        "DUMP/methods2.json",
        il2cpp::FUNCTIONS_TABLE_REFLECTION.get().unwrap(),
    )
}
