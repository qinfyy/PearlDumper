use crate::{il2cpp, module::{GAMEASSEMBLY_HANDLE, UNITYPLAYER_HANDLE}, output};
use colored::Colorize;
use ctor::ctor;
use figlet_rs::FIGfont;
use indicatif::{ProgressBar, ProgressStyle};
use std::{
    error::Error, thread, time::Duration, ptr::null_mut
};
use windows::{
    Win32::Foundation::HWND,
    Win32::Storage::FileSystem::{CreateFileW, FILE_GENERIC_WRITE, FILE_SHARE_WRITE, OPEN_EXISTING, FILE_GENERIC_READ, FILE_SHARE_READ},
    core::{w, PCWSTR},
    Win32::System::{
        Console::{AllocConsole, SetConsoleTitleW, SetStdHandle, SetConsoleMode, GetConsoleWindow, GetConsoleMode, CONSOLE_MODE,
                  ENABLE_PROCESSED_OUTPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING, ENABLE_WRAP_AT_EOL_OUTPUT, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE, STD_INPUT_HANDLE
        },
        LibraryLoader::GetModuleHandleW
    }
};

pub fn init() -> Result<(), Box<dyn Error>> {
    unsafe {
        if GetConsoleWindow() == HWND(null_mut()) {
            AllocConsole()?;
        }

        let console_output_handle = CreateFileW(w!("\\\\.\\CONOUT$"), FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
            FILE_SHARE_WRITE, None, OPEN_EXISTING, Default::default(), None)?;

        let console_input_handle = CreateFileW(w!("\\\\.\\CONIN$"), FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
            FILE_SHARE_READ, None, OPEN_EXISTING, Default::default(), None)?;

        SetStdHandle(STD_OUTPUT_HANDLE, console_output_handle)?;
        SetStdHandle(STD_ERROR_HANDLE, console_output_handle)?;
        SetStdHandle(STD_INPUT_HANDLE, console_input_handle)?;

        let mut mode = CONSOLE_MODE(0);
        GetConsoleMode(console_output_handle, &mut mode)?;
        mode |= ENABLE_VIRTUAL_TERMINAL_PROCESSING | ENABLE_PROCESSED_OUTPUT | ENABLE_WRAP_AT_EOL_OUTPUT;
        SetConsoleMode(console_output_handle, mode)?;

        let title = "Castorice Dumper";
        let wide: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
        let _ = SetConsoleTitleW(PCWSTR(wide.as_ptr()));

        let standard_font = FIGfont::standard()?;
        let figure = standard_font.convert("Castorice Dumper").unwrap();

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
    
        pb.finish_with_message("[+] All modules loaded");
        println!("\r");

        for i in (1..=16).rev() {
            print!("\r[*] Waiting {:2} seconds...   ", i);
            std::io::Write::flush(&mut std::io::stdout()).unwrap();
            thread::sleep(Duration::from_secs(1));
        }

        println!("\r[+] Done!                    ");

        il2cpp::init(*UNITYPLAYER_HANDLE, *GAMEASSEMBLY_HANDLE)?;

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
        init().unwrap_or_else(|e| panic!("castorice-dumper encountered an error: {}", e));
    });
}
