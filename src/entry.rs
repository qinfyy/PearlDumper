use crate::{il2cpp, module::{GAMEASSEMBLY_HANDLE, UNITYPLAYER_HANDLE}, output};
use colored::Colorize;
use ctor::ctor;
use figlet_rs::FIGfont;
use indicatif::{ProgressBar, ProgressStyle};
use std::{
    error::Error, thread, time::Duration,
};
use windows::{core::{w, PCWSTR}, Win32::System::{Console::{AllocConsole, SetConsoleTitleW}, LibraryLoader::GetModuleHandleW}};


fn init() -> Result<(), Box<dyn Error>> {
    unsafe {
        AllocConsole()?;
        let title = "Firefly Dumper";
        let wide: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
        let _ = SetConsoleTitleW(PCWSTR(wide.as_ptr()));

        let standard_font = FIGfont::standard().unwrap();
        let figure = standard_font.convert("Firefly Dumper").unwrap();

        println!("{}", figure.to_string().blue().bold());

        let pb = ProgressBar::new(2);
        pb.set_style(
            ProgressStyle::with_template(
                "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} • {msg}",
            )
            .unwrap()
            .progress_chars("#>-"),
        );
    
        pb.set_message("Waiting for GameAssembly...");
        while GetModuleHandleW(w!("GameAssembly")).is_err() {
            thread::sleep(Duration::from_millis(10));
        }
        pb.inc(1);

        pb.set_message("Waiting for UnityPlayer...");
        while GetModuleHandleW(w!("UnityPlayer")).is_err() {
            thread::sleep(Duration::from_millis(10));
        }
        pb.inc(1);
    
        pb.finish_with_message("✔ All modules loaded");

        println!("{}", "Setting...".green().bold());
        thread::sleep(Duration::from_secs(25));
        let _ = il2cpp::init(*UNITYPLAYER_HANDLE, *GAMEASSEMBLY_HANDLE);

        println!("{}", "Dumping...".green().bold());
        match output::init() {
            Ok(_) => {}
            Err(e) => eprintln!("Error: {}", e),
        }
    }
    Ok(())
}

#[ctor]
fn entry() {
    thread::spawn(|| {
        init().unwrap_or_else(|e| panic!("firefly-dumper encountered an error: {}", e));
    });
}
