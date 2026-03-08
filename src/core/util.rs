

#[macro_export]
macro_rules! cs_class {
    ($class_name:expr) => {
        #[inline(always)]
        pub fn get_class() -> anyhow::Result<$crate::il2cpp::api::Il2CppClass> {
            let Some(class) = $crate::il2cpp::get_cached_class($class_name) else {
                return Err(anyhow::anyhow!("no such class {}", $class_name));
            };

            Ok(class)
        }

        #[inline(always)]
        pub fn is_null(&self) -> bool {
            self.0 == 0
        }

        #[inline(always)]
        pub fn as_object(&self) -> $crate::il2cpp::native::Il2CppObject {
            $crate::il2cpp::native::Il2CppObject(self.0)
        }
    };
}

/// ```rust
/// cs_method!(pub set_target_framerate, "set_targetFrameRate", &["int"], (), (fps: i32));
/// ```
///
/// ```rust
/// cs_method!(pub to_string, "ToString", &[], Il2CppString, (), self);
/// ```
#[macro_export]
macro_rules! cs_method {
    // Static method version
    (
        $vis:vis $fn_name:ident,
        $method_name:expr,
        $arg_types:expr,
        $ret:ty,
        ($($arg_name:ident : $arg_ty:ty),*)
    ) => {
        #[inline(always)]
        $vis fn $fn_name($($arg_name: $arg_ty),*) -> anyhow::Result<$ret> {
            unsafe {
                let arg_types: &[&str] = $arg_types;
                let class = Self::get_class()?;

                let Some(method_info) = $crate::il2cpp::api::Il2CppClass::find_method(&class, $method_name, arg_types)
                else {
                    return Err(anyhow::anyhow!(
                        "No such static method {} in {}",
                        $method_name,
                        class.byval_arg().name()
                    ));
                };

                let func: extern "fastcall" fn($($arg_ty),*) -> $ret = std::mem::transmute(method_info.va());
                microseh::try_seh(||
                    func($($arg_name),*)
                )
                .map_err(|e|
                    anyhow::anyhow!(
                        "Runtime Exception! (static method call throws exception! type: {} method name: 0x{:X}): {:?}",
                        class.byval_arg().name(),
                        method_info.rva(),
                        e
                    )
                )
            }
        }
    };

    // Instance method version
    (
        $vis:vis $fn_name:ident,
        $method_name:expr,
        $arg_types:expr,
        $ret:ty,
        ($($arg_name:ident : $arg_ty:ty),*),
        self
    ) => {
        #[inline(always)]
        $vis fn $fn_name(&self, $($arg_name: $arg_ty),*) -> anyhow::Result<$ret> {
            unsafe {
                let arg_types: &[&str] = $arg_types;
                if self.0 == 0 {
                    return Err(anyhow::anyhow!(
                        "Null reference exception! Method: {}",
                        $method_name
                    ));
                }

                let object = $crate::il2cpp::native::Il2CppObject(self.0);
                let class = object.get_class();

                let Some(method_info) = $crate::il2cpp::api::Il2CppClass::find_method(&class, $method_name, arg_types)
                else {
                    return Err(anyhow::anyhow!(
                        "No such instance method {} in {}",
                        $method_name,
                        class.byval_arg().name()
                    ));
                };

                let func: extern "fastcall" fn(usize, $($arg_ty),*) -> $ret = std::mem::transmute(method_info.va());
                microseh::try_seh(||
                    func(self.0, $($arg_name),*)
                )
                .map_err(|e|
                    anyhow::anyhow!(
                        "Runtime Exception! (instance method call throws exception! type: {} method name: {}): {:?}",
                        class.byval_arg().name(),
                        method_info.name(),
                        e
                    )
                )
            }
        }
    };
}


#[macro_export]
macro_rules! cs_method_by_return_type {
    // Static method version
    (
        $vis:vis $fn_name:ident,
        $method_return:expr,
        $arg_types:expr,
        $ret:ty,
        ($($arg_name:ident : $arg_ty:ty),*)
    ) => {
        #[inline(always)]
        $vis fn $fn_name($($arg_name: $arg_ty),*) -> anyhow::Result<$ret> {
            unsafe {
                let arg_types: &[&str] = $arg_types;
                let class = Self::get_class()?;

                let Some(method_info) = $crate::il2cpp::api::Il2CppClass::find_method_by_return_type(&class, $method_return, arg_types)
                else {
                    return Err(anyhow::anyhow!(
                        "No such static method {} in {}",
                        $method_return,
                        class.byval_arg().name()
                    ));
                };

                let func: extern "fastcall" fn($($arg_ty),*) -> $ret = std::mem::transmute(method_info.va());
                microseh::try_seh(||
                    func($($arg_name),*)
                )
                .map_err(|e|
                    anyhow::anyhow!(
                        "Runtime Exception! (static method call throws exception! type: {} method name: 0x{:X}): {:?}",
                        class.byval_arg().name(),
                        method_info.rva(),
                        e
                    )
                )
            }
        }
    };

    // Instance method version
    (
        $vis:vis $fn_name:ident,
        $method_return:expr,
        $arg_types:expr,
        $ret:ty,
        ($($arg_name:ident : $arg_ty:ty),*),
        self
    ) => {
        #[inline(always)]
        $vis fn $fn_name(&self, $($arg_name: $arg_ty),*) -> anyhow::Result<$ret> {
            unsafe {
                let arg_types: &[&str] = $arg_types;
                if self.0 == 0 {
                    return Err(anyhow::anyhow!(
                        "Null reference exception! Method: {}",
                        $method_return
                    ));
                }

                let object = $crate::il2cpp::native::Il2CppObject(self.0);
                let class = object.get_class();

                let Some(method_info) = $crate::il2cpp::api::Il2CppClass::find_method_by_return_type(&class, $method_return, arg_types)
                else {
                    return Err(anyhow::anyhow!(
                        "No such instance method {} in {}",
                        $method_return,
                        class.byval_arg().name()
                    ));
                };

                let func: extern "fastcall" fn(usize, $($arg_ty),*) -> $ret = std::mem::transmute(method_info.va());
                microseh::try_seh(||
                    func(self.0, $($arg_name),*)
                )
                .map_err(|e|
                    anyhow::anyhow!(
                        "Runtime Exception! (instance method call throws exception! type: {} method name: {}): {:?}",
                        class.byval_arg().name(),
                        method_info.name(),
                        e
                    )
                )
            }
        }
    };
}


#[macro_export]
macro_rules! cs_method_custom {
    // Instance method version
    (
        $vis:vis $fn_name:ident,
        $method_name:expr,
        $arg_types:expr,
        $class_name:expr,
        $ret:ty,
        ($($arg_name:ident : $arg_ty:ty),*),
        self
    ) => {
        #[inline(always)]
        $vis fn $fn_name(&self, $($arg_name: $arg_ty),*) -> anyhow::Result<$ret> {
            unsafe {
                let arg_types: &[&str] = $arg_types;
                if self.0 == 0 {
                    return Err(anyhow::anyhow!(
                        "Null reference exception! Method: {}",
                        $method_name
                    ));
                }

                let Some(class) = $crate::il2cpp::get_cached_class($class_name)
                else {
                    return Err(anyhow::anyhow!(
                        "No such instance class: {}",
                        $class_name,
                    ));
                };

                let Some(method_info) = $crate::il2cpp::api::Il2CppClass::find_method(&class, $method_name, arg_types)
                else {
                    return Err(anyhow::anyhow!(
                        "No such instance method {} in {}",
                        $method_name,
                        class.byval_arg().name()
                    ));
                };

                let func: extern "fastcall" fn(usize, $($arg_ty),*) -> $ret = std::mem::transmute(method_info.va());
                microseh::try_seh(||
                    func(self.0, $($arg_name),*)
                )
                .map_err(|e|
                    anyhow::anyhow!(
                        "Runtime Exception! (instance method call throws exception! type: {} method name: {}): {:?}",
                        class.byval_arg().name(),
                        method_info.name(),
                        e
                    )
                )
            }
        }
    };
}


///
/// ```rust
/// cs_field!(a, "static");
/// ```
///
/// ```rust
/// cs_field!(b, "static transform", |field| -> u32 { 1 });
/// ```
///
/// ```rust
/// cs_field!(c, "instance", self);
/// ```
///
/// ```rust
/// cs_field!(d, "instance transform", self, |field| -> u32 { field.unbox::<u32>() });
/// ```
#[macro_export]
macro_rules! cs_field {
    // static field Il2CppObject
    ($ident:ident, $name:literal) => {
        $crate::cs_field!(@internal, $ident, $name, static, |value| -> $crate::il2cpp::native::Il2CppObject { value });
    };

    // instance field Il2CppObject
    ($ident:ident, $name:literal, self) => {
        $crate::cs_field!(@internal, $ident, $name, instance, |value| -> $crate::il2cpp::native::Il2CppObject { value });
    };

    // static field custom
    ($ident:ident, $name:literal, |$param:ident| -> $ret_ty:ty $block:block) => {
        $crate::cs_field!(@internal, $ident, $name, static, |$param| -> $ret_ty $block);
    };

    // instance field custom
    ($ident:ident, $name:literal, self, |$param:ident| -> $ret_ty:ty $block:block) => {
        $crate::cs_field!(@internal, $ident, $name, instance, |$param| -> $ret_ty $block);
    };

    // static field
    (@internal, $ident:ident, $name:literal, static, |$param:ident| -> $ret_ty:ty $block:block) => {
        paste::paste! {
            #[inline(always)]
            pub fn $ident() -> anyhow::Result<$ret_ty> {
                let class = $crate::core::runtime::RuntimeType::from_class(Self::get_class()?);
                let Ok(field_info) = class.get_field($name) else {
                    return Err(anyhow::anyhow!("no such static field {} in type {}", $name, class.get_il2cpp_type().name()));
                };

                let value = microseh::try_seh(|| {
                    let Some(value) = field_info.get_value_object($crate::il2cpp::native::Il2CppObject::NULL) else {
                        return Err(anyhow::anyhow!("field {} in {} is null", class.get_il2cpp_type().name(), $name))
                    };
                    Ok(value)
                })
                .map_err(|e| {
                    anyhow::anyhow!(
                        "Runtime Exception! failed to get static field value. type: {} name: {}. {:?}",
                        class.get_il2cpp_type().name(),
                        field_info.name(),
                        e
                    )
                })?;

                let $param: $crate::il2cpp::native::Il2CppObject = value?;
                Ok($block)
            }
        }
    };

    // instance field
    (@internal, $ident:ident, $name:literal, instance, |$param:ident| -> $ret_ty:ty $block:block) => {
        paste::paste! {
            #[inline(always)]
            pub fn $ident(&self) -> anyhow::Result<$ret_ty> {
                if self.0 == 0 {
                    return Err(anyhow::format_err!(
                        "object reference is not set to an instance of an object! method name: {}",
                        $name
                    ));
                }

                let class = $crate::core::runtime::RuntimeType::from_class(self.as_object().get_class());
                let Ok(field_info) = class.get_field($name) else {
                    return Err(anyhow::anyhow!("no such field {} in type {}", $name, class.get_il2cpp_type().name()));
                };

                let value = microseh::try_seh(|| {
                    let Some(value) = field_info.get_value_object($crate::il2cpp::native::Il2CppObject(self.0)) else {
                        return Err(anyhow::anyhow!("field {} in {} is null", class.get_il2cpp_type().name(), $name))
                    };
                    Ok(value)
                })
                .map_err(|e| {
                    anyhow::anyhow!(
                        "Runtime Exception! failed to get instance field value. type: {} name: {}. {:?}",
                        class.get_il2cpp_type().name(),
                        field_info.name(),
                        e
                    )
                })?;

                let $param: $crate::il2cpp::native::Il2CppObject = value?;
                Ok($block)
            }
        }
    };
}

#[macro_export]
macro_rules! cs_property {
    (
        $vis:vis $fn_name:ident,
        $getter_fn:expr,
        $ret:path,
        self
    ) => {
        paste::paste! {
           $crate::cs_method!(
                $vis [<get_ $fn_name>],
                $getter_fn,
                &[], $ret,
                (),
                self
            );
        }
    };

    (
        $vis:vis $fn_name:ident,
        $getter_fn:expr,
        enumtype $ret:ty,
        self
    ) => {
        $crate::cs_method!($fn_name, $getter_fn, &[], i32, (), self);

        paste::paste! {
            $vis fn [<get_ $fn_name>](&self) -> Option<$ret> {
                let obj = self.$fn_name().ok()?;
                #[allow(clippy::missing_transmute_annotations)]
                Some(unsafe { std::mem::transmute(obj) })
            }
        }
    };
}