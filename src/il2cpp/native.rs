use crate::il2cpp::api::{il2cpp_object_get_class, Il2CppClass};
use std::{ffi::c_void, ops::Add as _} ;

#[derive(Debug, Clone, Copy)]
#[repr(transparent)]
pub struct Il2CppObject(pub usize);

#[allow(unused)]
impl Il2CppObject {
    pub const NULL: Il2CppObject = Il2CppObject(0);
    #[inline(always)]
    pub fn new(ptr: usize) -> Self {
        Il2CppObject(ptr)
    }

    #[inline(always)]
    pub fn is_null(&self) -> bool {
        self.0 == 0
    }

    #[inline(always)]
    pub fn get_class(&self) -> Il2CppClass {
        unsafe { Il2CppClass(*(self.0 as *const usize)) }
    }

    #[inline(always)]
    pub fn unbox<T: Copy>(&self) -> T {
        unsafe { *((self.0 + 16) as *const T) }
    }
}

impl From<usize> for Il2CppObject {
    fn from(value: usize) -> Self {
        Self(value)
    }
}


#[derive(Debug, Copy, Clone)]
pub struct Il2CppArray(pub usize);

impl Il2CppArray {
    #[inline(always)]
    pub fn class(&self) -> Il2CppClass {
        unsafe { Il2CppClass(*(self.0 as *const usize)) }
    }

    #[inline(always)]
    pub fn monitor(&self) -> usize {
        unsafe { *((self.0 + 8) as *const usize) }
    }

    #[inline(always)]
    pub fn bounds(&self) -> usize {
        unsafe { *((self.0 + 16) as *const usize) }
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        unsafe { *((self.0 + 24) as *const usize) }
    }

    #[inline(always)]
    fn first_item_ptr(&self) -> usize {
        self.0 + 32
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline(always)]
    pub fn get<T>(&self, i: usize) -> &T {
        let size = std::mem::size_of::<T>();
        unsafe { &*((self.first_item_ptr().add(i * size)) as *const T) }
    }

    #[inline(always)]
    pub fn get_mut<T>(&mut self, i: usize) -> &mut T {
        let size = std::mem::size_of::<T>();
        unsafe { &mut *((self.first_item_ptr().add(i * size)) as *mut T) }
    }

    #[inline(always)]
    pub fn to_vec<T: Clone>(self) -> Vec<T> {
        unsafe {
            std::slice::from_raw_parts(self.first_item_ptr() as *const T, self.len()).to_vec()
        }
    }

    #[inline(always)]
    pub fn to_vec_sized<T: Clone>(self, size: i32) -> Vec<T> {
        unsafe {
            std::slice::from_raw_parts(self.first_item_ptr() as *const T, size as usize).to_vec()
        }
    }
}


#[derive(Debug, Copy, Clone)]
#[repr(transparent)]
pub struct NativeList(pub usize);

impl NativeList {
    #[inline(always)]
    pub fn class(&self) -> Il2CppClass {
        unsafe { Il2CppClass(*(self.0 as *const usize)) }
    }

    #[inline(always)]
    pub fn monitor(&self) -> usize {
        unsafe { *((self.0 + 8) as *const usize) }
    }

    #[inline(always)]
    pub fn items(&self) -> Il2CppArray {
        unsafe { Il2CppArray(*((self.0 + 16) as *const usize)) }
    }

    #[inline(always)]
    pub fn size(&self) -> i32 {
        unsafe { *((self.0 + 24) as *const i32) }
    }

    #[inline(always)]
    pub fn add(&self, item: Il2CppObject, class_name: &str) -> Result<(), anyhow::Error>{
        unsafe {
            let class_cf = il2cpp_object_get_class(Il2CppObject::new(self.0));
            let mt_get_dict = class_cf
                .find_method(
                    "Add",
                    &[class_name],
                )
                .ok_or(anyhow::anyhow!("no such method"))?;
    
            let func_gd: extern "fastcall" fn(usize, Il2CppObject) -> usize =
                std::mem::transmute(mt_get_dict.va());
    
            let _ = microseh::try_seh(|| func_gd(self.0, item)).map_err(|e| {
                anyhow::anyhow!(
                    "Runtime Exception! (type: {} method: {}): {:?}",
                    class_cf.byval_arg().name(),
                    mt_get_dict.name(),
                    e
                )
            })?;
        }
        Ok(())
    }
}

#[repr(C)]
pub struct NativeArray<T> {
    pub klass: *mut c_void,
    pub monitor: *mut c_void,
    pub bounds: *mut c_void,
    pub max_length: i32,
    pub vector: [T; 65535],
}

impl<T: PartialEq + Copy> NativeArray<T> {
    pub fn get(&self, index: usize) -> Option<T> {
        if index < self.max_length as usize {
            Some(self.vector[index])
        } else {
            None
        }
    }

    pub fn contains(&self, item: T) -> bool {
        for i in 0..(self.max_length as usize) {
            if self.vector[i] == item {
                return true;
            }
        }
        false
    }
}


#[repr(C)]
#[derive(Copy, Clone)]
pub struct Entry<TKey, TValue> {
    pub hash_code: i32,
    pub next: i32,
    pub key: TKey,
    pub value: TValue,
}

#[repr(C)]
pub struct NativeDictionary<TKey, TValue> {
    pub kass: *mut c_void,
    pub monitor: *mut c_void,
    pub buckets: *mut NativeArray<i32>,
    pub entries: *mut NativeArray<Entry<TKey, TValue>>,
    pub count: i32,
    pub version: i32,
    pub free_list: i32,
    pub free_count: i32,
    pub comparer: *mut c_void,
    pub keys: *mut KeysCollection<TKey, TValue>,
    pub values: *mut ValueCollection<TKey, TValue>,
    pub sync_root: *mut c_void,
}

impl<TKey: PartialEq + Copy, TValue: PartialEq + Copy + Default> NativeDictionary<TKey, TValue> {
    pub fn remove(dict: usize, key: usize, class_name: &str) -> Result<bool, anyhow::Error> {
        unsafe {
            let class_cf = il2cpp_object_get_class(Il2CppObject::new(dict));
            let mt_get_dict = class_cf
                .find_method(
                    "Remove",
                    &[class_name],
                )
                .ok_or(anyhow::anyhow!("no such method"))?;
    
                let func_gd: extern "fastcall" fn(usize, usize) -> bool =
                std::mem::transmute(mt_get_dict.va());
    
            let res = microseh::try_seh(|| func_gd(dict, key)).map_err(|e| {
                anyhow::anyhow!(
                    "Runtime Exception! (type: {} method: {}): {:?}",
                    class_cf.byval_arg().name(),
                    mt_get_dict.name(),
                    e
                )
            })?;
            Ok(res)
        }
    }
    pub fn get_comparer(&self) -> *mut c_void {
        self.comparer
    }

    pub fn get_count(&self) -> i32 {
        self.count
    }

    pub fn find_entry(&self, key: TKey) -> i32 {
        unsafe {
            if let Some(entries) = self.entries.as_ref() {
                for i in 0..self.count {
                    if entries.vector[i as usize].key == key {
                        return i;
                    }
                }
            }
            -1
        }
    }

    pub fn contains_key(&self, key: TKey) -> bool {
        self.find_entry(key) >= 0
    }

    pub fn contains_value(&self, value: TValue) -> bool {
        unsafe {
            if let Some(entries) = self.entries.as_ref() {
                for i in 0..self.count {
                    let entry = entries.vector[i as usize];
                    if entry.hash_code >= 0 && entry.value == value {
                        return true;
                    }
                }
            }
            false
        }
    }

    pub fn try_get_value(&self, key: TKey) -> Option<TValue> {
        let i = self.find_entry(key);
        unsafe {
            if i >= 0 {
                Some((*self.entries).vector[i as usize].value)
            } else {
                None
            }
        }
    }

    pub fn get_value_or_default(&self, key: TKey) -> TValue {
        self.try_get_value(key).unwrap_or_default()
    }

    pub fn get_item(&self, key: TKey) -> TValue {
        let i = self.find_entry(key);
        unsafe {
            if i >= 0 {
                return (*self.entries).vector[i as usize].value;
            }
            TValue::default()
        }
    }
}
#[repr(C)]
pub struct KeysCollection<TKey, TValue> {
    pub dictionary: *mut NativeDictionary<TKey, TValue>,
}

impl<TKey: Copy + PartialEq + Default, TValue: Copy + PartialEq + Default> KeysCollection<TKey, TValue> {
    pub fn get(&self, index: usize) -> TKey {
        unsafe {
            let dict = self.dictionary.as_ref().unwrap();
            if let Some(entries) = dict.entries.as_ref() {
                return entries.vector[index].key;
            }
            TKey::default()
        }
    }

    pub fn get_count(&self) -> i32 {
        unsafe { self.dictionary.as_ref().unwrap().get_count() }
    }
}

#[repr(C)]
pub struct ValueCollection<TKey, TValue> {
    pub dictionary: *mut NativeDictionary<TKey, TValue>,
}

impl<TKey: Copy + PartialEq, TValue: Copy + PartialEq + Default> ValueCollection<TKey, TValue> {
    pub fn get(&self, index: usize) -> TValue {
        unsafe {
            let dict = self.dictionary.as_ref().unwrap();
            if let Some(entries) = dict.entries.as_ref() {
                return entries.vector[index].value;
            }
            TValue::default()
        }
    }

    pub fn get_count(&self) -> i32 {
        unsafe { self.dictionary.as_ref().unwrap().get_count() }
    }
}