use std::collections::HashMap;
use std::fs;
use std::path::Path;

use rayon::prelude::*;
use serde::Serialize;

use crate::error::Result;
use crate::metadata::Metadata;

mod builder;
mod heaps;
mod pe;
mod tables;

use builder::AsmBuilder;
pub const ATTRIBUTE_ASSEMBLY: &str = "Il2CppDummyDll";

#[derive(Clone, Default, Serialize)]
pub struct DummyDllReport {
    pub assemblies: usize,
    pub fields: usize,
    pub constants: usize,
    pub parameters: usize,
    pub parameter_constants: usize,
    pub initialized_field_data_not_emitted: usize,
}

struct Context<'a> {
    metadata: &'a Metadata,
    assembly_names: Vec<String>,
    type_to_image: Vec<usize>,
    generic_instantiations: HashMap<usize, Vec<(u64, String)>>,
}

impl<'a> Context<'a> {
    fn build(metadata: &'a Metadata) -> Result<Self> {
        let images = metadata.images();
        let assembly_names = images
            .iter()
            .map(|image| {
                image
                    .name
                    .strip_suffix(".dll")
                    .unwrap_or(&image.name)
                    .to_owned()
            })
            .collect();

        let type_count = images
            .iter()
            .map(|image| image.type_start + image.type_count)
            .max()
            .unwrap_or(0);
        let mut type_to_image = vec![0usize; type_count];
        for (image_index, image) in images.iter().enumerate() {
            let end = (image.type_start + image.type_count).min(type_count);
            for slot in &mut type_to_image[image.type_start.min(type_count)..end] {
                *slot = image_index;
            }
        }

        Ok(Self {
            metadata,
            assembly_names,
            type_to_image,
            generic_instantiations: crate::script::collect_generic_instantiations(metadata)?,
        })
    }
}

pub fn build_dummy_dll(metadata: &Metadata, out_dir: &Path) -> Result<usize> {
    Ok(build_dummy_dll_with_report(metadata, out_dir)?.assemblies)
}

pub fn build_dummy_dll_with_report(metadata: &Metadata, out_dir: &Path) -> Result<DummyDllReport> {
    let ctx = Context::build(metadata)?;
    fs::create_dir_all(out_dir)?;

    fs::write(
        out_dir.join(format!("{ATTRIBUTE_ASSEMBLY}.dll")),
        builder::build_attribute_assembly(),
    )?;

    let stats = (0..ctx.metadata.images().len())
        .into_par_iter()
        .map(|image_index| -> Result<DummyDllReport> {
            let (bytes, stats) = AsmBuilder::build_image(&ctx, image_index).map_err(|error| {
                error.context(format!(
                    "DummyDll assembly {}",
                    ctx.assembly_names[image_index]
                ))
            })?;
            let path = out_dir.join(format!("{}.dll", ctx.assembly_names[image_index]));
            fs::write(path, bytes)?;
            Ok(stats)
        })
        .collect::<Result<Vec<_>>>()?;

    let mut report = DummyDllReport {
        assemblies: ctx.metadata.images().len() + 1,
        ..DummyDllReport::default()
    };
    for image in stats {
        report.fields += image.fields;
        report.constants += image.constants;
        report.parameters += image.parameters;
        report.parameter_constants += image.parameter_constants;
        report.initialized_field_data_not_emitted += image.initialized_field_data_not_emitted;
    }
    Ok(report)
}
