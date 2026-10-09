use std::collections::BTreeSet;

use crate::proto::recovery::{Evidence, Hints};

// Output vocabulary for the verified ServerDispatchData sinks, not wire tags.
fn dispatch_name(name: &str) -> Option<(&'static str, &'static str)> {
    Some(match name.trim_start_matches('_') {
        "name" => ("region_name", "string"),
        "login_white_msg" => ("msg", "string"),
        "design_data_bundle_version_update_url"
        | "design_data_bundle_version_update_url_with_suffix" => ("ex_resource_url", "string"),
        "asset_bundle_version_update_url" | "asset_bundle_version_update_url_with_suffix" => {
            ("asset_bundle_url", "string")
        }
        "base_asset_bundle_version_update_url" => ("asset_bundle_url_android", "string"),
        "lua_bundle_version_update_url" | "lua_bundle_version_update_url_with_suffix" => {
            ("lua_url", "string")
        }
        "i_fix_patch_version_update_url" => ("ifix_url", "string"),
        "i_fix_patch_revision" => ("ifix_version", "string"),
        "lua_patch_version" => ("mdk_res_version", "string"),
        "port" => ("port", "uint32"),
        "use_tcp" => ("use_tcp", "bool"),
        "ios_exam" => ("ios_exam", "bool"),
        "enable_asset_bundle_version_update" => ("enable_asset_bundle_version_update", "bool"),
        _ => return None,
    })
}

pub(super) fn normalize(hints: &mut Hints, kind: impl Fn(&str) -> Option<String>) {
    let asset_sources: BTreeSet<_> = hints
        .entries
        .iter()
        .filter(|c| {
            c.evidence == Evidence::Native
                && dispatch_name(&c.name) == Some(("asset_bundle_url", "string"))
                && kind(&c.symbol).as_deref() == Some("string")
        })
        .map(|c| c.symbol.clone())
        .collect();
    let asset_enable_sources: BTreeSet<_> = hints
        .entries
        .iter()
        .filter(|c| {
            c.evidence == Evidence::Native
                && c.name.trim_start_matches('_') == "enable_asset_bundle_version_update"
                && kind(&c.symbol).as_deref() == Some("bool")
        })
        .map(|c| c.symbol.clone())
        .collect();
    let ios_sources: BTreeSet<_> = hints
        .entries
        .iter()
        .filter(|c| {
            c.evidence == Evidence::Native
                && c.name.trim_start_matches('_') == "ios_exam"
                && kind(&c.symbol).as_deref() == Some("bool")
        })
        .map(|c| c.symbol.clone())
        .collect();
    for c in &mut hints.entries {
        if c.evidence != Evidence::Native {
            continue;
        }
        let alias = dispatch_name(&c.name)
            .or_else(|| {
                (ios_sources.contains(&c.symbol)
                    && c.name.trim_start_matches('_') == "is_ios_approval_package")
                    .then_some(("ios_exam", "bool"))
            })
            .or_else(|| {
                (asset_enable_sources.contains(&c.symbol)
                    && matches!(
                        c.name.trim_start_matches('_'),
                        "enable_audio_bundle_version_update"
                            | "enable_video_bundle_version_update"
                            | "enable_lua_bundle_version_update"
                    ))
                .then_some(("enable_asset_bundle_version_update", "bool"))
            })
            .or_else(|| {
                // Audio/video share the ASB source in these clients. Only fold their
                // names when a copy from this SAME wire field to the ASB sink exists.
                (asset_sources.contains(&c.symbol)
                    && matches!(
                        c.name.trim_start_matches('_'),
                        "audio_bundle_version_update_url"
                            | "audio_bundle_version_update_url_with_suffix"
                            | "video_bundle_version_update_url"
                            | "video_bundle_version_update_url_with_suffix"
                    ))
                .then_some(("asset_bundle_url", "string"))
            });
        if let Some((name, expected)) = alias {
            if kind(&c.symbol).as_deref() != Some(expected) {
                c.evidence = Evidence::Hint;
                c.source.push_str("; GateServer sink/wire type mismatch");
            } else if c.name != name {
                c.source.push_str(&format!(
                    "; GateServer output alias {} -> {name} (client sink retained in original_name)",
                    c.name
                ));
                c.name = name.into();
            }
        }
    }
}
