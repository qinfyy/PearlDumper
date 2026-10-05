use crate::proto::recovery::Hints;
use std::{borrow::Cow, sync::LazyLock};

use reflection::runtime_type::RuntimeType;

const TARGET_PROPERTY: &str = "System.Collections.Generic.IEnumerator<System.ValueTuple<RPG.GameCore.RelicType,RPG.AvatarSystem.IRelicProxy>>.Current";

pub static AVATAR_HANDLER_CLASS: LazyLock<Cow<'static, str>> = LazyLock::new(|| {
    let classes = il2cpp::CLASS_TABLE_VEC.get().expect("missing class cache");
    for &class in &classes[unsafe { il2cpp::ASSEMBLY_CSHARP_START } as usize..] {
        if !class
            .get_fields()
            .iter()
            .any(|field| field.get_name() == "uniqueData")
        {
            continue;
        }
        let Ok(rt) = RuntimeType::from_class(class) else {
            continue;
        };
        let has_prop = rt
            .get_properties(62)
            .iter()
            .any(|p| p.get_name().is_ok_and(|n| n.as_str() == TARGET_PROPERTY));
        if !has_prop {
            continue;
        }
        let name = rt.il_name();
        if let Some(stripped) = name.split('.').next() {
            let result = stripped.to_string();
            log::debug!("[Method NT] AvatarHandlerClass => {result}");
            return Cow::Owned(result);
        }
    }

    log::debug!("[Method NT] failed to find AvatarHandlerClass");
    panic!("unresolved client layout; see preceding log entry");
});

pub fn get_avatar_nt() -> Hints {
    let mut map = Hints::new();
    let handler_class_name = &*AVATAR_HANDLER_CLASS;
    let Some(class) = il2cpp::get_cached_class(handler_class_name) else {
        return map;
    };
    let Ok(rt) = RuntimeType::from_class(class) else {
        return map;
    };

    for method in rt.get_methods_il2cpp() {
        let params = method.get_parameters();
        if params.len() == 1
            && let Some(param_type) = super::non_null_type(params[0].get_parameter_type())
            && let Ok(return_type) = method.get_return_type()
            && return_type.il_name().contains("IEnumerable")
        {
            let obf_name = param_type.il_name().into_owned();
            if obf_name.len() == 11 && obf_name.chars().all(char::is_uppercase) {
                map.insert(obf_name, "AvatarPathData".to_string());
            }
        }
        if params.len() == 2
            && let Some(p2_type) = super::non_null_type(params[1].get_parameter_type())
        {
            let obf_name = p2_type.il_name().into_owned();
            if obf_name.len() == 11 && obf_name.chars().all(char::is_uppercase) {
                log::debug!("[Method NT] param nt: {obf_name} -> Avatar");
                map.insert(obf_name, "Avatar".to_string());
            }
        }
    }
    if map.is_empty() {
        log::debug!("[Method NT] failed to find avatar nt");
    }
    map
}

pub fn get_avatar_response_nt() -> Hints {
    super::get_callback_parameter_nt(&[
        ("<LevelUpAvatar", "AvatarExpUpScRsp"),
        ("<UnlockTraceNode", "UnlockSkilltreeScRsp"),
        ("<PromoteAvatar", "PromoteAvatarScRsp"),
        ("<EquipLightCone", "DressAvatarScRsp"),
        ("<TakeOffLightCone", "TakeOffEquipmentScRsp"),
        ("<ActiveEidolon", "RankUpAvatarScRsp"),
        ("<EquipRelics", "DressRelicAvatarScRsp"),
        ("<TakeOffRelics", "TakeOffRelicScRsp"),
        ("<TakePromotionReward", "TakePromotionRewardScRsp"),
        ("<DressSkin", "DressAvatarSkinScRsp"),
        ("<TakeOffSkin", "TakeOffAvatarSkinScRsp"),
        ("<SwitchAcivePathes", "SetMultipleAvatarPathsScRsp"),
        ("<SetGrowthTarget", "SetGrowthTargetAvatarScRsp"),
        ("<FlipIsMarked", "MarkAvatarScRsp"),
        ("<UnlockAvatarPath", "UnlockAvatarPathScRsp"),
    ])
}
