mod claudian;

use crate::{
    errors::{AppError, AppResult},
    models::{
        PluginAdapterSettingChange, PluginAdapterSettingField, PluginInventoryItem,
        PluginSettingsAdapterInfo, PluginSettingsAdapterStatus,
    },
};
use semver::{Version, VersionReq};
use serde_json::{Map, Value};

pub(crate) const CLAUDIAN_ADAPTER_ID: &str = "builtin.claudian.cli-paths";

#[derive(Debug, Clone)]
pub(crate) struct AdapterHostContext {
    pub legacy_hostname: Option<String>,
}

impl AdapterHostContext {
    fn system() -> Self {
        let legacy_hostname = std::env::var("COMPUTERNAME")
            .or_else(|_| std::env::var("HOSTNAME"))
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        Self { legacy_hostname }
    }
}

pub(crate) struct AdapterRegistration {
    pub id: &'static str,
    pub name: &'static str,
    pub plugin_id: &'static str,
    pub version_requirement: &'static str,
    pub inspect: fn(&Value, &AdapterHostContext) -> Vec<PluginAdapterSettingField>,
    pub apply: fn(
        &Value,
        &[PluginAdapterSettingChange],
        &AdapterHostContext,
    ) -> AppResult<AdapterApplyResult>,
    pub device_local_paths: &'static [&'static [&'static str]],
}

#[derive(Debug, Clone)]
pub struct AdapterApplyResult {
    pub configuration: Value,
    pub changed_fields: Vec<String>,
}

const ADAPTERS: &[AdapterRegistration] = &[claudian::REGISTRATION];

pub fn inspect_plugin_adapter(
    plugin: &PluginInventoryItem,
    configuration: Option<&Value>,
) -> Option<PluginSettingsAdapterInfo> {
    inspect_plugin_adapter_with_host(plugin, configuration, &AdapterHostContext::system())
}

pub fn apply_plugin_adapter_changes(
    plugin: &PluginInventoryItem,
    configuration: &Value,
    adapter_id: &str,
    changes: &[PluginAdapterSettingChange],
) -> AppResult<AdapterApplyResult> {
    apply_plugin_adapter_changes_with_host(
        plugin,
        configuration,
        adapter_id,
        changes,
        &AdapterHostContext::system(),
    )
}

pub fn should_filter_configuration_for_sync(
    source_plugin: &PluginInventoryItem,
    target_plugin: Option<&PluginInventoryItem>,
    copy_plugin_files: bool,
) -> bool {
    sync_registration(source_plugin, target_plugin, copy_plugin_files).is_some()
}

pub fn prepare_configuration_for_sync(
    source_plugin: &PluginInventoryItem,
    target_plugin: Option<&PluginInventoryItem>,
    copy_plugin_files: bool,
    source_configuration: &Value,
    target_configuration: Option<&Value>,
) -> AppResult<Option<Value>> {
    let Some(registration) = sync_registration(source_plugin, target_plugin, copy_plugin_files)
    else {
        return Ok(None);
    };
    if !source_configuration.is_object()
        || target_configuration.is_some_and(|configuration| !configuration.is_object())
    {
        return Err(AppError::new(
            "plugin_adapter_sync_configuration_invalid",
            "适配插件的 data.json 顶层必须是 JSON 对象，已停止同步以保护设备本地设置",
        ));
    }

    let mut next = source_configuration.clone();
    for path in registration.device_local_paths {
        if let Some(value) = target_configuration.and_then(|target| value_at_path(target, path)) {
            set_value_at_path(&mut next, path, value.clone())?;
        } else {
            remove_value_at_path(&mut next, path);
        }
    }
    Ok(Some(next))
}

pub fn configurations_equal_for_sync(
    source_plugin: &PluginInventoryItem,
    target_plugin: &PluginInventoryItem,
    source_configuration: &Value,
    target_configuration: &Value,
) -> AppResult<Option<bool>> {
    let Some(registration) = sync_registration(source_plugin, Some(target_plugin), false) else {
        return Ok(None);
    };
    if !source_configuration.is_object() || !target_configuration.is_object() {
        return Err(AppError::new(
            "plugin_adapter_sync_configuration_invalid",
            "适配插件的 data.json 顶层必须是 JSON 对象，无法安全比较设备本地设置",
        ));
    }
    let mut source_portable = source_configuration.clone();
    let mut target_portable = target_configuration.clone();
    for path in registration.device_local_paths {
        remove_value_at_path(&mut source_portable, path);
        remove_value_at_path(&mut target_portable, path);
    }
    Ok(Some(source_portable == target_portable))
}

fn sync_registration(
    source_plugin: &PluginInventoryItem,
    target_plugin: Option<&PluginInventoryItem>,
    copy_plugin_files: bool,
) -> Option<&'static AdapterRegistration> {
    let source = compatible_registration(source_plugin)?;
    if copy_plugin_files {
        return Some(source);
    }
    let target = compatible_registration(target_plugin?)?;
    (source.id == target.id).then_some(source)
}

fn compatible_registration(plugin: &PluginInventoryItem) -> Option<&'static AdapterRegistration> {
    let plugin_id = plugin.id.as_deref()?;
    let registration = ADAPTERS
        .iter()
        .find(|registration| registration.plugin_id == plugin_id)?;
    plugin
        .version
        .as_deref()
        .is_some_and(|version| version_matches(version, registration.version_requirement))
        .then_some(registration)
}

fn value_at_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter()
        .try_fold(value, |current, segment| current.as_object()?.get(*segment))
}

fn set_value_at_path(value: &mut Value, path: &[&str], next: Value) -> AppResult<()> {
    let Some((last, parents)) = path.split_last() else {
        return Err(AppError::new(
            "plugin_adapter_path_invalid",
            "适配器设备本地路径不能为空",
        ));
    };
    let mut current = value.as_object_mut().ok_or_else(|| {
        AppError::new(
            "plugin_adapter_sync_configuration_invalid",
            "适配插件的 data.json 顶层必须是 JSON 对象",
        )
    })?;
    for segment in parents {
        let child = current
            .entry((*segment).to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        current = child.as_object_mut().ok_or_else(|| {
            AppError::new(
                "plugin_adapter_sync_configuration_invalid",
                format!("设备本地路径父字段 {segment} 必须是 JSON 对象"),
            )
        })?;
    }
    current.insert((*last).to_string(), next);
    Ok(())
}

fn remove_value_at_path(value: &mut Value, path: &[&str]) -> bool {
    let Some((first, rest)) = path.split_first() else {
        return false;
    };
    let Some(object) = value.as_object_mut() else {
        return false;
    };
    if rest.is_empty() {
        return object.remove(*first).is_some();
    }
    let Some(child) = object.get_mut(*first) else {
        return false;
    };
    let removed = remove_value_at_path(child, rest);
    if removed && child.as_object().is_some_and(Map::is_empty) {
        object.remove(*first);
    }
    removed
}

fn apply_plugin_adapter_changes_with_host(
    plugin: &PluginInventoryItem,
    configuration: &Value,
    adapter_id: &str,
    changes: &[PluginAdapterSettingChange],
    host: &AdapterHostContext,
) -> AppResult<AdapterApplyResult> {
    let plugin_id = plugin
        .id
        .as_deref()
        .ok_or_else(|| AppError::new("adapter_plugin_id_missing", "插件缺少可验证的 ID"))?;
    let registration = ADAPTERS
        .iter()
        .find(|registration| registration.plugin_id == plugin_id)
        .ok_or_else(|| {
            AppError::new(
                "plugin_adapter_unavailable",
                "该插件没有受信任的内置设置适配器",
            )
        })?;
    if registration.id != adapter_id {
        return Err(AppError::new(
            "plugin_adapter_mismatch",
            "提交的适配器与当前插件不匹配",
        ));
    }
    let version = plugin.version.as_deref().ok_or_else(|| {
        AppError::new(
            "plugin_adapter_version_missing",
            "插件版本未知，不能使用内置适配器写入",
        )
    })?;
    if !version_matches(version, registration.version_requirement) {
        return Err(AppError::new(
            "plugin_adapter_version_mismatch",
            format!(
                "插件版本 {version} 不在适配器支持范围 {} 内",
                registration.version_requirement
            ),
        ));
    }
    if changes.is_empty() {
        return Err(AppError::new(
            "plugin_adapter_changes_empty",
            "没有需要保存的适配设置",
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    if let Some(duplicate) = changes
        .iter()
        .find(|change| !seen.insert(change.field_id.as_str()))
    {
        return Err(AppError::new(
            "plugin_adapter_duplicate_field",
            format!("适配设置重复提交：{}", duplicate.field_id),
        ));
    }

    (registration.apply)(configuration, changes, host)
}

fn inspect_plugin_adapter_with_host(
    plugin: &PluginInventoryItem,
    configuration: Option<&Value>,
    host: &AdapterHostContext,
) -> Option<PluginSettingsAdapterInfo> {
    let plugin_id = plugin.id.as_deref()?;
    let registration = ADAPTERS
        .iter()
        .find(|registration| registration.plugin_id == plugin_id)?;
    let compatible = plugin
        .version
        .as_deref()
        .is_some_and(|version| version_matches(version, registration.version_requirement));
    let status = if compatible {
        PluginSettingsAdapterStatus::Compatible
    } else {
        PluginSettingsAdapterStatus::VersionMismatch
    };
    let mut warnings = Vec::new();
    let fields = if compatible {
        let empty = Value::Object(Default::default());
        (registration.inspect)(configuration.unwrap_or(&empty), host)
    } else {
        warnings.push(format!(
            "已安装版本 {} 不在适配器支持范围 {} 内，适配设置已禁用",
            plugin.version.as_deref().unwrap_or("未知"),
            registration.version_requirement
        ));
        Vec::new()
    };

    Some(PluginSettingsAdapterInfo {
        id: registration.id.to_string(),
        name: registration.name.to_string(),
        plugin_id: registration.plugin_id.to_string(),
        installed_version: plugin.version.clone(),
        version_requirement: registration.version_requirement.to_string(),
        status,
        fields,
        warnings,
    })
}

fn version_matches(version: &str, requirement: &str) -> bool {
    let Ok(requirement) = VersionReq::parse(requirement) else {
        return false;
    };
    let Ok(version) = Version::parse(version.trim().trim_start_matches('v')) else {
        return false;
    };
    requirement.matches(&version)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{PluginSettingPortability, UnsupportedReason};
    use serde_json::json;

    fn plugin(id: &str, version: Option<&str>) -> PluginInventoryItem {
        PluginInventoryItem {
            id: Some(id.to_string()),
            folder_name: id.to_string(),
            folder_path: format!("C:/vault/.obsidian/plugins/{id}"),
            manifest_path: format!("C:/vault/.obsidian/plugins/{id}/manifest.json"),
            name: Some(id.to_string()),
            version: version.map(str::to_string),
            enabled: true,
            has_data_json: true,
            valid: true,
            unsupported_reason: None::<UnsupportedReason>,
            warnings: Vec::new(),
        }
    }

    fn host() -> AdapterHostContext {
        AdapterHostContext {
            legacy_hostname: Some("WORKSTATION".to_string()),
        }
    }

    #[test]
    fn unknown_plugins_do_not_receive_adapter_behavior() {
        assert!(inspect_plugin_adapter_with_host(
            &plugin("unknown-plugin", Some("1.0.0")),
            Some(&json!({})),
            &host(),
        )
        .is_none());
    }

    #[test]
    fn version_mismatch_disables_adapter_fields() {
        let result = inspect_plugin_adapter_with_host(
            &plugin("realclaudian", Some("2.1.0")),
            Some(&json!({})),
            &host(),
        )
        .unwrap();

        assert_eq!(result.status, PluginSettingsAdapterStatus::VersionMismatch);
        assert!(result.fields.is_empty());
        assert!(result.warnings[0].contains("2.1.0"));
    }

    #[test]
    fn compatible_claudian_exposes_device_local_cli_fields() {
        let result = inspect_plugin_adapter_with_host(
            &plugin("realclaudian", Some("2.0.24")),
            Some(&json!({})),
            &host(),
        )
        .unwrap();

        assert_eq!(result.status, PluginSettingsAdapterStatus::Compatible);
        assert_eq!(result.fields.len(), 4);
        assert!(result.fields.iter().all(|field| {
            field.portability == PluginSettingPortability::DeviceLocal && field.writable
        }));
    }

    #[test]
    fn adapter_write_revalidates_identity_and_version() {
        let change = PluginAdapterSettingChange {
            field_id: "claudian.cli-path.claude".to_string(),
            value: json!(""),
        };
        let wrong_adapter = apply_plugin_adapter_changes_with_host(
            &plugin("realclaudian", Some("2.0.24")),
            &json!({}),
            "builtin.other",
            std::slice::from_ref(&change),
            &host(),
        )
        .expect_err("adapter identity must be checked");
        assert_eq!(wrong_adapter.code, "plugin_adapter_mismatch");

        let wrong_version = apply_plugin_adapter_changes_with_host(
            &plugin("realclaudian", Some("2.1.0")),
            &json!({}),
            CLAUDIAN_ADAPTER_ID,
            &[change],
            &host(),
        )
        .expect_err("adapter version must be checked");
        assert_eq!(wrong_version.code, "plugin_adapter_version_mismatch");
    }

    #[test]
    fn portable_comparison_ignores_device_local_values() {
        let source_plugin = plugin("realclaudian", Some("2.0.24"));
        let target_plugin = plugin("realclaudian", Some("2.0.21"));
        let source = json!({
            "locale": "zh-cn",
            "providerConfigs": {
                "claude": {
                    "safeMode": "auto",
                    "cliPath": "C:/source-legacy.exe",
                    "cliPathsByHost": {"device:source": "C:/source.exe"}
                }
            }
        });
        let target = json!({
            "locale": "zh-cn",
            "providerConfigs": {
                "claude": {
                    "safeMode": "auto",
                    "cliPath": "D:/target-legacy.exe",
                    "cliPathsByHost": {"device:target": "D:/target.exe"}
                }
            }
        });

        assert_eq!(
            configurations_equal_for_sync(&source_plugin, &target_plugin, &source, &target,)
                .unwrap(),
            Some(true)
        );
    }

    #[test]
    fn portable_comparison_keeps_normal_settings_significant() {
        let source_plugin = plugin("realclaudian", Some("2.0.24"));
        let target_plugin = plugin("realclaudian", Some("2.0.24"));
        let source = json!({"locale": "zh-cn"});
        let target = json!({"locale": "en"});

        assert_eq!(
            configurations_equal_for_sync(&source_plugin, &target_plugin, &source, &target,)
                .unwrap(),
            Some(false)
        );
    }

    #[test]
    fn sync_preserves_target_device_values() {
        let source_plugin = plugin("realclaudian", Some("2.0.24"));
        let target_plugin = plugin("realclaudian", Some("2.0.24"));
        let source = json!({
            "locale": "zh-cn",
            "providerConfigs": {
                "claude": {
                    "safeMode": "auto",
                    "cliPathsByHost": {"device:source": "C:/source.exe"}
                }
            }
        });
        let target = json!({
            "locale": "en",
            "providerConfigs": {
                "claude": {
                    "safeMode": "default",
                    "cliPathsByHost": {"device:target": "D:/target.exe"}
                }
            }
        });

        let merged = prepare_configuration_for_sync(
            &source_plugin,
            Some(&target_plugin),
            false,
            &source,
            Some(&target),
        )
        .unwrap()
        .unwrap();
        assert_eq!(merged.pointer("/locale"), Some(&json!("zh-cn")));
        assert_eq!(
            merged.pointer("/providerConfigs/claude/safeMode"),
            Some(&json!("auto"))
        );
        assert_eq!(
            merged.pointer("/providerConfigs/claude/cliPathsByHost"),
            Some(&json!({"device:target": "D:/target.exe"}))
        );
    }

    #[test]
    fn new_target_drops_source_device_values() {
        let source_plugin = plugin("realclaudian", Some("2.0.24"));
        let source = json!({
            "locale": "zh-cn",
            "providerConfigs": {
                "pi": {
                    "enabled": true,
                    "cliPathsByHost": {"device:source": "C:/pi.exe"}
                }
            }
        });

        let merged = prepare_configuration_for_sync(&source_plugin, None, true, &source, None)
            .unwrap()
            .unwrap();
        assert!(merged
            .pointer("/providerConfigs/pi/cliPathsByHost")
            .is_none());
        assert_eq!(
            merged.pointer("/providerConfigs/pi/enabled"),
            Some(&json!(true))
        );
    }

    #[test]
    fn version_mismatch_disables_sync_filtering() {
        let source_plugin = plugin("realclaudian", Some("2.0.24"));
        let target_plugin = plugin("realclaudian", Some("2.1.0"));

        assert!(!should_filter_configuration_for_sync(
            &source_plugin,
            Some(&target_plugin),
            false,
        ));
    }
}
