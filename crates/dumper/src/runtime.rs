use std::sync::Mutex;

static RUNTIME_INIT: Mutex<bool> = Mutex::new(false);

pub fn init_default() -> anyhow::Result<()> {
    log::debug!("Acquiring runtime initialization lock");
    let mut ready = RUNTIME_INIT
        .lock()
        .map_err(|_| anyhow::anyhow!("runtime initialization was interrupted"))?;
    if *ready {
        log::info!("Runtime already initialized");
        return Ok(());
    }
    log::info!("Runtime step 1/2: IL2CPP initialization and method map");
    il2cpp::init_il2cpp(false)?;
    log::info!("Runtime step 2/2: reflection bindings, checks and method map");
    reflection::init_reflection_methods()?;
    *ready = true;
    log::info!("Runtime initialization finished");
    Ok(())
}

pub fn attach_current_thread_to_il2cpp() {
    let domain = il2cpp::api::il2cpp_domain_get();
    il2cpp::vm::thread::Il2CppThread::attach(domain);
}
