use std::error::Error;

use crate::il2cpp::get_cached_class;

#[repr(transparent)]
#[derive(Debug, Clone)]
pub struct ExcelTable {
    pub key: String,
}

impl ExcelTable {
    pub fn new(key: &str) -> Self {
        ExcelTable {
            key: String::from(key),
        }
    }

    pub fn cctor(&self) -> Result<(), Box<dyn Error>> {
        unsafe {
            let class_cf = get_cached_class(&self.key).ok_or(anyhow::anyhow!("no such class"))?;
            let mt_cctor = class_cf
                .find_method(".cctor", &[])
                .ok_or(anyhow::anyhow!("no such method"))?;

            let func_cctor: extern "fastcall" fn() -> () =
                std::mem::transmute(mt_cctor.va());

            let _cctor = microseh::try_seh(|| func_cctor()).map_err(|e| {
                anyhow::anyhow!(
                    "Runtime Exception! (type: {} method: {}): {:?}",
                    class_cf.byval_arg().name(),
                    mt_cctor.name(),
                    e
                )
            });

            let cctor = match _cctor {
                Ok(cctor) => cctor,
                Err(e) => return Err(e.into()),
            };

            Ok(cctor)
        }
    }

    pub fn get_excel_table(&self) -> Result<usize, Box<dyn Error>> {
        unsafe {
            let class_cf = get_cached_class(&self.key).ok_or(anyhow::anyhow!("no such class"))?;
            let mt_get_dict = class_cf
                .find_method_by_return_type("System.Collections.Generic.Dictionary<", &[])
                .ok_or(anyhow::anyhow!("no such method"))?;

            let func_gd: extern "fastcall" fn() -> usize = std::mem::transmute(mt_get_dict.va());

            let _dict_ptr = microseh::try_seh(|| func_gd()).map_err(|e| {
                anyhow::anyhow!(
                    "Runtime Exception! (type: {} method: {}): {:?}",
                    class_cf.byval_arg().name(),
                    mt_get_dict.name(),
                    e
                )
            });

            let dict_ptr = match _dict_ptr {
                Ok(dict_ptr) => dict_ptr,
                Err(e) => return Err(e.into()),
            };

            Ok(dict_ptr)
        }
    }

    pub fn is_data_empty(&self) -> Result<bool, Box<dyn Error>> {
        unsafe {
            let class_cf = get_cached_class(&self.key).ok_or(anyhow::anyhow!("no such class"))?;
            let mt_is_empty = class_cf
                .find_method("IsDataEmpty", &[])
                .or_else(|| class_cf.find_method("MBALLPDOBKM", &[])) //3.8.0
                .or_else(|| class_cf.find_method("IMIDPJAINOL", &[])) //4.0.0
                .or_else(|| class_cf.find_method("KMLBGAIEAML", &[])) //4.0.5x
                .ok_or(anyhow::anyhow!("no such method"))?;

            let func_is_empty: extern "fastcall" fn() -> bool =
                std::mem::transmute(mt_is_empty.va());

            let _is_empty = microseh::try_seh(|| func_is_empty()).map_err(|e| {
                anyhow::anyhow!(
                    "Runtime Exception! (type: {} method: {}): {:?}",
                    class_cf.byval_arg().name(),
                    mt_is_empty.name(),
                    e
                )
            });

            let is_empty = match _is_empty {
                Ok(is_empty) => is_empty,
                Err(e) => return Err(e.into()),
            };

            Ok(is_empty)
        }
    }

    pub fn is_data_loaded(&self) -> Result<bool, Box<dyn Error>> {
        unsafe {
            let class_cf = get_cached_class(&self.key).ok_or(anyhow::anyhow!("no such class"))?;
            let mt_is_loaded = class_cf
                .find_method("IsDataLoaded", &[])
                .or_else(|| class_cf.find_method("FKLAGBMGOJN", &[])) //3.8.0
                .or_else(|| class_cf.find_method("KOJCECFHOIC", &[])) //4.0.0
                .or_else(|| class_cf.find_method("HAOHLGBMOOI", &[])) //4.0.5x
                .ok_or(anyhow::anyhow!("no such method"))?;

            let func_is_loaded: extern "fastcall" fn() -> bool =
                std::mem::transmute(mt_is_loaded.va());

            let _is_loaded = microseh::try_seh(|| func_is_loaded()).map_err(|e| {
                anyhow::anyhow!(
                    "Runtime Exception! (type: {} method: {}): {:?}",
                    class_cf.byval_arg().name(),
                    mt_is_loaded.name(),
                    e
                )
            });

            let is_loaded = match _is_loaded {
                Ok(is_loaded) => is_loaded,
                Err(e) => return Err(e.into()),
            };

            Ok(is_loaded)
        }
    }

    pub fn load_data(&self) -> Result<(), Box<dyn Error>> {
        unsafe {
            let class_cf = get_cached_class(&self.key).ok_or(anyhow::anyhow!("no such class"))?;
            let mt_load_data = class_cf
                .find_method("LoadData", &[])
                .or_else(|| class_cf.find_method("DHKGNMNOHJB", &[])) //3.8.0
                .or_else(|| class_cf.find_method("HOIPHJHEAIC", &[])) //4.0.0
                .or_else(|| class_cf.find_method("NNCCJOMJBJE", &[])) //4.0.5x
                .ok_or(anyhow::anyhow!("no such method"))?;

            let func_load_data: extern "fastcall" fn() -> () =
                std::mem::transmute(mt_load_data.va());

            let _load_data = microseh::try_seh(|| func_load_data()).map_err(|e| {
                anyhow::anyhow!(
                    "Runtime Exception! (type: {} method: {}): {:?}",
                    class_cf.byval_arg().name(),
                    mt_load_data.name(),
                    e
                )
            });

            let load_data = match _load_data {
                Ok(load_data) => load_data,
                Err(e) => return Err(e.into()),
            };

            Ok(load_data)
        }
    }

    pub fn unload_data(&self) -> Result<(), Box<dyn Error>> {
        unsafe {
            let class_cf = get_cached_class(&self.key).ok_or(anyhow::anyhow!("no such class"))?;
            let mt_unload_data = class_cf
                .find_method("UnloadData", &[])
                .or_else(|| class_cf.find_method("DHKGNMNOHJB", &[]))//3.8.0
                .or_else(|| class_cf.find_method("MDMNPDGLJED", &[]))//4.0.0
                .or_else(|| class_cf.find_method("BLIMEEAKBKC", &[]))//4.0.5x
                .ok_or(anyhow::anyhow!("no such method"))?;

            let func_unload_data: extern "fastcall" fn() -> () =
                std::mem::transmute(mt_unload_data.va());

            let _unload_data = microseh::try_seh(|| func_unload_data()).map_err(|e| {
                anyhow::anyhow!(
                    "Runtime Exception! (type: {} method: {}): {:?}",
                    class_cf.byval_arg().name(),
                    mt_unload_data.name(),
                    e
                )
            });

            let unload_data = match _unload_data {
                Ok(unload_data) => unload_data,
                Err(e) => return Err(e.into()),
            };

            Ok(unload_data)
        }
    }
}
