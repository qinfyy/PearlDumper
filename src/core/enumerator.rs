use crate::{cs_method, il2cpp::{api::il2cpp_object_get_class, native::Il2CppObject}};

#[repr(transparent)]
#[derive(Debug, Copy, Clone)]
pub struct SystemEnumerator(pub usize);

impl SystemEnumerator {
    cs_method!(pub move_next, "MoveNext", &[], bool, (), self);
    
    pub fn move_next_2(&self) -> Result<bool, anyhow::Error>{
        unsafe {
            let class_cf = il2cpp_object_get_class(Il2CppObject::new(self.0));

            let mt_mv_next = class_cf
                .find_method(
                    "MoveNext",
                    &[],
                )
                .ok_or(anyhow::anyhow!("no such method"))?;

            let func_mv_next: extern "fastcall" fn(usize) -> bool =
                std::mem::transmute(mt_mv_next.va());

            let res = microseh::try_seh(|| func_mv_next(self.0)).map_err(|e| {
                anyhow::anyhow!(
                    "Runtime Exception! (type: {} method: {}): {:?}",
                    class_cf.byval_arg().name(),
                    mt_mv_next.name(),
                    e
                )
            })?;
            Ok(res)
        }
    }

    pub fn new(ptr: usize) -> Self {
        SystemEnumerator(ptr)
    }
}