use crate::{
    core::{string::SystemString, system_type::SystemType},
    cs_class, cs_method, cs_method_custom, cs_property,
    il2cpp::{
        api::{Il2CppClass, Il2CppField, Il2CppType}, constants, get_cached_class, native::{Il2CppArray, Il2CppObject}
    },
};

#[repr(transparent)]
#[derive(Debug, Copy, Clone)]
pub struct RuntimeType(pub usize);

impl RuntimeType {
    cs_class!(&String::from("System.RuntimeType"));

    cs_property!(pub base_type, "get_BaseType", RuntimeType, self);
    cs_property!(pub is_generic_type, "get_IsGenericType", bool,  self);
    cs_property!(pub is_enum, "get_IsEnum", bool, self);
    cs_property!(pub array_rank, "GetArrayRank", usize, self);
    cs_property!(pub reflected_type, "get_ReflectedType", RuntimeType, self);
    cs_property!(pub element_type, "GetElementType", RuntimeType, self);
    cs_property!(pub namespace, "get_Namespace", SystemString, self);
    cs_property!(pub name, "get_Name", SystemString, self);
    cs_property!(pub full_name, "get_FullName", SystemString, self);
    cs_property!(pub type_handle, "get_TypeHandle", Il2CppObject, self);
    cs_property!(pub generic_arguments, "GetGenericArguments", Il2CppArray, self);
    cs_method!(pub get_fields, "GetFields", &["System.Reflection.BindingFlags"], Il2CppArray, (binding_flags: i32), self);
    cs_method!(pub get_properties, "GetProperties", &["System.Reflection.BindingFlags"], Il2CppArray, (binding_flags: i32), self);
    cs_method_custom!(pub get_is_by_ref, "get_IsByRef", &[], &String::from("System.Type"), bool, (), self);
    cs_method_custom!(pub get_is_array, "get_IsArray", &[], &String::from("System.Type"), bool, (), self);
    cs_method_custom!(pub get_is_value_type, "get_IsValueType", &[], &String::from("System.Type"), bool, (), self);
    cs_method_custom!(pub get_is_pointer, "get_IsPointer", &[], &String::from("System.Type"), bool, (), self);
    cs_method_custom!(pub get_type, "GetType", &[], &String::from("System.Type"), usize, (), self);
    cs_method_custom!(pub get_property, "GetProperty", &["string"], &String::from("System.Type"), MonoProperty, (name: SystemString), self);
    cs_method_custom!(pub get_is_interface, "get_IsInterface", &[], &String::from("System.Type"), bool, (), self);

    cs_method!(_get_field, "GetField", &["string", "System.Reflection.BindingFlags"], MonoField, (name: SystemString, binding_flags: i32), self);
    #[inline]
    pub fn new(ptr: usize) -> Self {
        RuntimeType(ptr)
    }

    #[inline(always)]
    pub fn get_field(&self, name: &str) -> anyhow::Result<Il2CppField> {
        match self._get_field(name.into(), 60) {
            Ok(field) => {
                if field.0 != 0 {
                    return Ok(field.get_il2cpp_field());
                } else {
                    let base_type = self.get_base_type()?;
                    let field = base_type._get_field(name.into(), 60)?;
                    if field.0 != 0 {
                        return Ok(field.get_il2cpp_field());
                    }
                }
            }
            Err(_) => {
                let base_type = self.get_base_type()?;
                let field = base_type._get_field(name.into(), 60)?;
                if field.0 != 0 {
                    return Ok(field.get_il2cpp_field());
                }
            }
        }

        Err(anyhow::format_err!(
            "no such field {} in {}",
            name,
            self.get_il2cpp_type().name()
        ))
    }

    pub fn get_field_special(&self) -> anyhow::Result<Vec<MonoField>> {
        let parent = self.get_il2cpp_type().get_class().get_parent();
        let mut fields: Vec<MonoField> = Vec::new();
        if parent.0 != 0
        {
            let parent_fields = self.get_base_type()?.get_field_special()?;
            fields.extend(parent_fields);
        }
        let mut child_fields: Vec<MonoField> = Vec::new();
        let child_fields_array = self.get_fields(22)?;
        for i in 0..child_fields_array.len() {
            let field = child_fields_array.get::<MonoField>(i);
            let flags = field.get_il2cpp_field().get_flags();
            if flags & constants::FIELD_ATTRIBUTE_PUBLIC != 0 {
                child_fields.push(field.clone());
            }
        }

        let metadata_tokens: Vec<i32> = child_fields.iter().map(|field| field.get_metadata_token().unwrap()).collect();

        let mut fields_met: Vec<(MonoField, i32)> = child_fields.into_iter().zip(metadata_tokens.into_iter()).collect();
             
        fields_met.sort_by_key(|&(_, token)| token);

 
        let (mut new_fields, _): (Vec<MonoField>, Vec<i32>) = fields_met.into_iter().unzip();
        
        fields.append(&mut new_fields);

        Ok(fields)
    }
    #[inline(always)]
    pub fn from_class(class: Il2CppClass) -> Self {
        Self(
            SystemType::get_type_from_handle(class.byval_arg())
                .unwrap()
                .0,
        )
    }

    #[inline(always)]
    pub fn from_name(name: &String) -> Self {
        Self::from_class(get_cached_class(name).unwrap())
    }

    #[inline(always)]
    pub fn get_il2cpp_type(&self) -> Il2CppType {
        unsafe { Il2CppType(*((self.0 + 16) as *const usize)) }
    }
}

#[repr(transparent)]
#[derive(Debug, Copy, Clone)]
pub struct MonoField(pub usize);

impl MonoField {
    cs_property!(pub declaring_type, "get_DeclaringType", RuntimeType, self);
    cs_property!(pub field_type, "get_FieldType", RuntimeType, self);
    cs_property!(pub name, "get_Name", SystemString, self);
    cs_property!(pub field_handle, "get_FieldHandle", RuntimeFieldHandle, self);
    cs_property!(pub raw_constant_value, "GetRawConstantValue", usize, self);
    cs_method!(pub get_custom_attribute, "GetCustomAttributes", &["bool"], Il2CppArray, (value: bool), self);
    cs_method!(pub get_value, "GetValue", &["object"], Il2CppObject, (value: usize), self);
    cs_method_custom!(pub get_is_literal, "get_IsLiteral", &[], &String::from("System.Reflection.FieldInfo"), bool, (), self);
    cs_method_custom!(pub get_metadata_token, "get_MetadataToken", &[], &String::from("System.Reflection.MemberInfo"), i32, (), self);
    cs_method_custom!(pub set_value, "SetValue", &["object", "object"], &String::from("System.Reflection.FieldInfo"), (), (obj: usize, value: usize), self);
    #[inline(always)]
    pub fn get_il2cpp_field(&self) -> Il2CppField {
        unsafe { Il2CppField(*((self.0 + 24) as *const usize)) }
    }

    #[inline]
    pub fn new(ptr: usize) -> Self {
        MonoField(ptr)
    }
}
#[repr(transparent)]
#[derive(Debug, Copy, Clone)]
pub struct RuntimeFieldHandle(pub usize);

impl RuntimeFieldHandle {
    cs_property!(pub value, "get_Value", usize, self);
}

#[derive(Debug, Copy, Clone)]
pub struct MonoProperty(pub usize);

impl MonoProperty {
    cs_property!(pub declaring_type, "get_DeclaringType", RuntimeType, self);
    cs_property!(pub property_type, "get_PropertyType", RuntimeType, self);
    cs_property!(pub name, "get_Name", SystemString, self);
    cs_method_custom!(pub set_value, "SetValue", &["object", "object"], &String::from("System.Reflection.PropertyInfo"), (), (class: usize, value: usize), self);
    cs_method_custom!(pub get_value, "GetValue", &["object"], &String::from("System.Reflection.PropertyInfo"), Il2CppObject, (class: usize), self);
    cs_method!(pub get_value_2, "GetValue", &["object", "System.Object[]"], usize, (class: usize, args: usize),self);
    #[inline]
    pub fn new(ptr: usize) -> Self {
        MonoProperty(ptr)
    }
}
