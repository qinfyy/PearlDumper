use std::{
    ffi::c_void,
    panic::{AssertUnwindSafe, catch_unwind},
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::{CloseHandle, HINSTANCE, HMODULE},
        System::{
            Console::{AllocConsole, SetConsoleTitleW},
            LibraryLoader::{DisableThreadLibraryCalls, GetModuleFileNameW, GetModuleHandleW},
            SystemServices::DLL_PROCESS_ATTACH,
            Threading::{CreateThread, THREAD_CREATION_FLAGS},
        },
    },
    core::w,
};

fn run(module: *mut c_void) -> anyhow::Result<()> {
    unsafe {
        let _ = AllocConsole();
        let _ = SetConsoleTitleW(w!("Castorice Dumper 4.6.51"));
    }
    crate::logging::init()?;
    tracing::info!("Castorice Dumper: target 4.6.51");
    let build = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    tracing::info!(
        "Build={build}, pid={}, cwd={}",
        std::process::id(),
        std::env::current_dir()?.display()
    );
    let mut path = vec![0u16; 32768];
    let length = unsafe { GetModuleFileNameW(Some(HMODULE(module)), &mut path) } as usize;
    if length > 0 && length < path.len() {
        tracing::info!(
            "Loaded dumper: {}, base=0x{:X}",
            String::from_utf16_lossy(&path[..length]),
            module as usize
        );
    } else {
        tracing::warn!(
            "Cannot resolve dumper module path at 0x{:X}",
            module as usize
        );
    }
    if cfg!(debug_assertions) {
        tracing::warn!(
            "Debug build: large metadata scans may be slow; use a release build for full exports"
        );
    }
    let started = Instant::now();
    let mut last_progress = started;
    tracing::info!("Waiting for GameAssembly.dll and UnityPlayer.dll (timeout 120s)");
    while unsafe { GetModuleHandleW(w!("GameAssembly.dll")) }.is_err()
        || unsafe { GetModuleHandleW(w!("UnityPlayer.dll")) }.is_err()
    {
        anyhow::ensure!(
            started.elapsed() < Duration::from_secs(120),
            "timed out waiting for game modules"
        );
        if last_progress.elapsed() >= Duration::from_secs(2) {
            tracing::info!(
                "Still waiting for game modules after {:.1}s",
                started.elapsed().as_secs_f64()
            );
            last_progress = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    tracing::info!(
        "Game modules loaded in {:.2}s",
        started.elapsed().as_secs_f64()
    );
    dumper::run()?;
    tracing::info!("Dump finished; output: DUMP");
    Ok(())
}

unsafe extern "system" fn worker(module: *mut c_void) -> u32 {
    let result = microseh::try_seh(|| catch_unwind(AssertUnwindSafe(|| run(module))));
    match result {
        Ok(Ok(Ok(()))) => 0,
        Ok(Ok(Err(error))) => {
            tracing::error!("Stopped: {error:#}");
            1
        }
        Ok(Err(_)) => {
            tracing::error!("Stopped after a Rust panic");
            1
        }
        Err(error) => {
            tracing::error!("Stopped after a native exception: {error:?}");
            1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllMain(instance: HINSTANCE, reason: u32, _: *mut c_void) -> i32 {
    if reason == DLL_PROCESS_ATTACH {
        unsafe {
            let _ = DisableThreadLibraryCalls(instance.into());
            match CreateThread(
                None,
                0,
                Some(worker),
                Some(instance.0),
                THREAD_CREATION_FLAGS(0),
                None,
            ) {
                Ok(handle) => {
                    let _ = CloseHandle(handle);
                }
                Err(_) => return 0,
            }
        }
    }
    1
}
