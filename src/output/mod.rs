use std::{error::Error};

use colored::Colorize;

use crate::{core::{app_domain::AppDomain, assembly::MonoAssembly}, output::{config::config_dump, excel::excel_dump, proto::proto_dump}};

pub mod excel;
pub mod proto;
pub mod config;
pub mod util;

pub fn init() -> Result<(), Box<dyn Error>> {
    let domain = AppDomain::get_current_domain()?;
    let assemblies = domain.get_assemblies()?;

    let mut proto_assembly = None;
    let mut excel_assembly = None;
    for i in 0..assemblies.len() {
        if proto_assembly.is_some() && excel_assembly.is_some() {
            break;
        }

        let assembly: &usize = assemblies.get(i);
        let mono_assembly = MonoAssembly::new(*assembly);
        let assembly_name = mono_assembly.get_full_name()?.as_str();

        if assembly_name.starts_with("RPG.Network.Proto") {
            proto_assembly = Some(mono_assembly.clone());
        } else if assembly_name.starts_with("RPG.GameCore.Config") {
            excel_assembly = Some(mono_assembly.clone());
        }

    }

    println!("{}", "✅ Start dumping proto".green().bold());
    if let Some(proto_assembly) = proto_assembly {
        match proto_dump(&proto_assembly) {
            Ok(_) => println!("{}", "✅ Proto dumping success".green().bold()),
            Err(e) => println!("{} {}", "Proto dumping error:".red().bold(), e),
        }
    }

    println!("{}", "✅ Start dumping excel".green().bold());
    if let Some(excel_assembly) = excel_assembly {
        match excel_dump(&excel_assembly) {
            Ok(_) => println!("{}", "✅ Excel dumping success".green().bold()),
            Err(e) => println!("{} {}", "Excel dumping error:".red().bold(), e),
        }
    }

    println!("{}", "✅ Start dumping config".green().bold());
    match config_dump() {
        Ok(_) => println!("{}", "✅ Config dumping success".green().bold()),
        Err(e) => println!("{} {}", "Config dumping error:".red().bold(), e),
    };

    Ok(())
}