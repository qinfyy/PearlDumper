use std::{env, path::PathBuf, time::Instant};

use anyhow::{Result, bail};

fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let Some(game_dir) = args.next() else {
        bail!("usage: morax GAME_DIR [OUTPUT_DIR] [--inspect]");
    };
    let game_dir = PathBuf::from(game_dir);
    let mut output = None;
    let mut inspect = false;
    for arg in args {
        if arg == "--inspect" {
            inspect = true;
        } else if arg.to_string_lossy().starts_with('-') {
            bail!("unknown option: {}", arg.to_string_lossy());
        } else if output.is_none() {
            output = Some(PathBuf::from(arg));
        } else {
            bail!("unexpected argument: {}", arg.to_string_lossy());
        }
    }
    let started = Instant::now();
    if inspect {
        let metadata = morax::export::load_game(&game_dir)?;
        println!(
            "Loaded in {:.2}s: {:#?}",
            started.elapsed().as_secs_f64(),
            metadata.header()
        );
        for image in metadata.images() {
            println!("{}: {} types", image.name, image.type_count);
        }
        return Ok(());
    }
    let output = output.unwrap_or_else(|| game_dir.join("DUMP/Morax"));
    morax::export::export_game(&game_dir, &output)?;
    println!(
        "Completed in {:.2}s: {}",
        started.elapsed().as_secs_f64(),
        output.display()
    );
    Ok(())
}
