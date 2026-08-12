use super::{AdapterApplyResult, AdapterHostContext, AdapterRegistration, CLAUDIAN_ADAPTER_ID};
use crate::{
    errors::{AppError, AppResult},
    fs_safety,
    models::{
        PluginAdapterSettingChange, PluginAdapterSettingField, PluginSettingControl,
        PluginSettingPortability,
    },
};
use serde_json::{Map, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};

const PROVIDERS: &[(&str, &str)] = &[
    ("claude", "Claude"),
    ("codex", "Codex"),
    ("opencode", "OpenCode"),
    ("pi", "Pi"),
];

pub(super) const DEVICE_LOCAL_PATHS: &[&[&str]] = &[
    &["providerConfigs", "claude", "cliPath"],
    &["providerConfigs", "claude", "cliPathsByHost"],
    &["providerConfigs", "codex", "cliPath"],
    &["providerConfigs", "codex", "cliPathsByHost"],
    &["providerConfigs", "codex", "installationMethod"],
    &["providerConfigs", "codex", "installationMethodsByHost"],
    &["providerConfigs", "codex", "wslDistroOverride"],
    &["providerConfigs", "codex", "wslDistroOverridesByHost"],
    &["providerConfigs", "opencode", "cliPath"],
    &["providerConfigs", "opencode", "cliPathsByHost"],
    &["providerConfigs", "pi", "cliPath"],
    &["providerConfigs", "pi", "cliPathsByHost"],
    &["claudeCliPath"],
    &["claudeCliPathsByHost"],
    &["codexCliPath"],
    &["codexCliPathsByHost"],
];

pub(super) const REGISTRATION: AdapterRegistration = AdapterRegistration {
    id: CLAUDIAN_ADAPTER_ID,
    name: "Claudian CLI 路径适配器",
    plugin_id: "realclaudian",
    version_requirement: ">=2.0.21, <2.1.0",
    inspect: inspect_fields,
    apply: apply_changes,
    device_local_paths: DEVICE_LOCAL_PATHS,
};

fn apply_changes(
    configuration: &Value,
    changes: &[PluginAdapterSettingChange],
    host: &AdapterHostContext,
) -> AppResult<AdapterApplyResult> {
    if !configuration.is_object() {
        return Err(AppError::new(
            "plugin_adapter_configuration_invalid",
            "Claudian data.json 顶层必须是 JSON 对象",
        ));
    }
    let mut next = configuration.clone();
    let mut changed_fields = Vec::new();

    for change in changes {
        let provider_id = provider_id_for_field(&change.field_id).ok_or_else(|| {
            AppError::new(
                "plugin_adapter_field_unknown",
                format!("适配器不支持字段：{}", change.field_id),
            )
        })?;
        let raw_value = change.value.as_str().ok_or_else(|| {
            AppError::new(
                "plugin_adapter_value_invalid",
                format!("{} 必须是文件路径文本", change.field_id),
            )
        })?;
        let normalized_path = normalize_cli_path(raw_value)?;
        let existing_paths = provider_configuration(&next, provider_id)
            .and_then(|provider| provider.get("cliPathsByHost"))
            .and_then(Value::as_object);
        let host_key = match select_host_key(existing_paths, host.legacy_hostname.as_deref()) {
            HostSelection::Existing { key, .. } => key,
            HostSelection::LegacyMigration { hostname } => hostname,
            HostSelection::Unavailable { warning } => {
                return Err(AppError::new("plugin_adapter_device_ambiguous", warning))
            }
        };

        if let Some(path) = normalized_path {
            ensure_cli_paths_mut(&mut next, provider_id)?.insert(host_key, Value::String(path));
        } else if let Some(paths) = existing_cli_paths_mut(&mut next, provider_id) {
            paths.remove(&host_key);
        }
        changed_fields.push(change.field_id.clone());
    }

    Ok(AdapterApplyResult {
        configuration: next,
        changed_fields,
    })
}

fn provider_id_for_field(field_id: &str) -> Option<&'static str> {
    PROVIDERS.iter().find_map(|(provider_id, _)| {
        (field_id == format!("claudian.cli-path.{provider_id}")).then_some(*provider_id)
    })
}

fn normalize_cli_path(value: &str) -> AppResult<Option<String>> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let path = expand_home_path(trimmed);
    if !path.is_absolute() {
        return Err(
            AppError::new("plugin_adapter_path_not_absolute", "CLI 路径必须是绝对路径")
                .with_path(path),
        );
    }
    if !path.exists() {
        return Err(AppError::new("plugin_adapter_path_missing", "CLI 文件不存在").with_path(path));
    }
    if fs_safety::is_link_path(&path)? {
        return Err(
            AppError::new("unsupported_link_path", "CLI 路径不能是链接或重解析点").with_path(path),
        );
    }
    let metadata = fs::metadata(&path).map_err(|error| AppError::from(error).with_path(&path))?;
    if !metadata.is_file() {
        return Err(
            AppError::new("plugin_adapter_path_not_file", "CLI 路径必须指向文件").with_path(path),
        );
    }
    let canonical = fs_safety::canonical_existing(&path)?;
    Ok(Some(storable_path(&canonical)))
}

fn expand_home_path(value: &str) -> PathBuf {
    let is_home_relative = value == "~"
        || value
            .strip_prefix('~')
            .is_some_and(|suffix| suffix.starts_with('/') || suffix.starts_with('\\'));
    if !is_home_relative {
        return PathBuf::from(value);
    }
    let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) else {
        return PathBuf::from(value);
    };
    let suffix = value
        .trim_start_matches('~')
        .trim_start_matches(['/', '\\']);
    PathBuf::from(home).join(suffix)
}

fn storable_path(path: &Path) -> String {
    let value = path.to_string_lossy();
    if let Some(unc) = value.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{unc}");
    }
    value.strip_prefix(r"\\?\").unwrap_or(&value).to_string()
}

fn ensure_cli_paths_mut<'a>(
    configuration: &'a mut Value,
    provider_id: &str,
) -> AppResult<&'a mut Map<String, Value>> {
    let root = configuration.as_object_mut().ok_or_else(|| {
        AppError::new(
            "plugin_adapter_configuration_invalid",
            "Claudian data.json 顶层必须是 JSON 对象",
        )
    })?;
    let providers = ensure_object_member(root, "providerConfigs")?;
    let provider = ensure_object_member(providers, provider_id)?;
    ensure_object_member(provider, "cliPathsByHost")
}

fn existing_cli_paths_mut<'a>(
    configuration: &'a mut Value,
    provider_id: &str,
) -> Option<&'a mut Map<String, Value>> {
    configuration
        .get_mut("providerConfigs")?
        .get_mut(provider_id)?
        .get_mut("cliPathsByHost")?
        .as_object_mut()
}

fn ensure_object_member<'a>(
    object: &'a mut Map<String, Value>,
    key: &str,
) -> AppResult<&'a mut Map<String, Value>> {
    let value = object
        .entry(key.to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    value.as_object_mut().ok_or_else(|| {
        AppError::new(
            "plugin_adapter_configuration_invalid",
            format!("Claudian 配置字段 {key} 必须是 JSON 对象"),
        )
    })
}

fn inspect_fields(
    configuration: &Value,
    host: &AdapterHostContext,
) -> Vec<PluginAdapterSettingField> {
    PROVIDERS
        .iter()
        .map(|(provider_id, provider_name)| {
            inspect_cli_path_field(configuration, host, provider_id, provider_name)
        })
        .collect()
}

fn inspect_cli_path_field(
    configuration: &Value,
    host: &AdapterHostContext,
    provider_id: &str,
    provider_name: &str,
) -> PluginAdapterSettingField {
    let cli_paths = provider_configuration(configuration, provider_id)
        .and_then(|provider| provider.get("cliPathsByHost"))
        .and_then(Value::as_object);
    let selection = select_host_key(cli_paths, host.legacy_hostname.as_deref());
    let (value, writable, warnings) = match selection {
        HostSelection::Existing { key, warning } => {
            let value = cli_paths
                .and_then(|paths| paths.get(&key))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            (Value::String(value), true, warning.into_iter().collect())
        }
        HostSelection::LegacyMigration { hostname } => (
            Value::String(String::new()),
            true,
            vec![format!(
                "保存后先写入主机键 {hostname}，Claudian 下次启动时会迁移到当前不透明设备键"
            )],
        ),
        HostSelection::Unavailable { warning } => {
            (Value::String(String::new()), false, vec![warning])
        }
    };

    PluginAdapterSettingField {
        id: format!("claudian.cli-path.{provider_id}"),
        name: format!("{provider_name} CLI 路径"),
        description: Some("仅作用于当前 Windows 设备，不参与普通跨库同步".to_string()),
        control: PluginSettingControl::Text,
        options: Vec::new(),
        value,
        default_value: Some(Value::String(String::new())),
        portability: PluginSettingPortability::DeviceLocal,
        writable,
        warnings,
    }
}

fn provider_configuration<'a>(
    configuration: &'a Value,
    provider_id: &str,
) -> Option<&'a Map<String, Value>> {
    configuration
        .get("providerConfigs")?
        .get(provider_id)?
        .as_object()
}

enum HostSelection {
    Existing {
        key: String,
        warning: Option<String>,
    },
    LegacyMigration {
        hostname: String,
    },
    Unavailable {
        warning: String,
    },
}

fn select_host_key(
    cli_paths: Option<&Map<String, Value>>,
    legacy_hostname: Option<&str>,
) -> HostSelection {
    if let Some(hostname) = legacy_hostname {
        if cli_paths.is_some_and(|paths| paths.contains_key(hostname)) {
            return HostSelection::Existing {
                key: hostname.to_string(),
                warning: Some("正在编辑 Claudian 的旧主机键；下次启动会自动迁移".to_string()),
            };
        }
    }

    let device_keys = cli_paths
        .into_iter()
        .flat_map(|paths| paths.keys())
        .filter(|key| key.starts_with("device:"))
        .cloned()
        .collect::<Vec<_>>();
    if device_keys.len() == 1 {
        return HostSelection::Existing {
            key: device_keys[0].clone(),
            warning: Some("检测到单一不透明设备键，按当前安装的设备设置处理".to_string()),
        };
    }
    if device_keys.len() > 1 {
        return HostSelection::Unavailable {
            warning: "检测到多个不透明设备键，无法静态证明哪一个属于当前 Obsidian 安装".to_string(),
        };
    }
    if let Some(hostname) = legacy_hostname {
        return HostSelection::LegacyMigration {
            hostname: hostname.to_string(),
        };
    }
    HostSelection::Unavailable {
        warning: "无法读取 Windows 主机名，不能创建 Claudian 兼容迁移键".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::PluginAdapterSettingChange;
    use serde_json::json;
    use std::{
        env,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn host(name: Option<&str>) -> AdapterHostContext {
        AdapterHostContext {
            legacy_hostname: name.map(str::to_string),
        }
    }

    fn temp_path(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        env::temp_dir().join(format!("obsidian-plugin-adapter-{name}-{unique}"))
    }

    fn change(provider: &str, value: Value) -> PluginAdapterSettingChange {
        PluginAdapterSettingChange {
            field_id: format!("claudian.cli-path.{provider}"),
            value,
        }
    }

    #[test]
    fn uses_existing_legacy_hostname_entry() {
        let fields = inspect_fields(
            &json!({
                "providerConfigs": {
                    "claude": {"cliPathsByHost": {"WORKSTATION": "C:/claude.cmd"}}
                }
            }),
            &host(Some("WORKSTATION")),
        );

        assert_eq!(fields[0].value, json!("C:/claude.cmd"));
        assert!(fields[0].writable);
    }

    #[test]
    fn uses_single_opaque_device_entry() {
        let fields = inspect_fields(
            &json!({
                "providerConfigs": {
                    "codex": {"cliPathsByHost": {"device:only": "C:/codex.exe"}}
                }
            }),
            &host(Some("WORKSTATION")),
        );

        assert_eq!(fields[1].value, json!("C:/codex.exe"));
        assert!(fields[1].writable);
    }

    #[test]
    fn multiple_opaque_devices_remain_read_only() {
        let fields = inspect_fields(
            &json!({
                "providerConfigs": {
                    "pi": {"cliPathsByHost": {
                        "device:first": "C:/pi-one.exe",
                        "device:second": "C:/pi-two.exe"
                    }}
                }
            }),
            &host(Some("WORKSTATION")),
        );

        assert!(!fields[3].writable);
        assert!(fields[3].warnings[0].contains("多个"));
    }

    #[test]
    fn first_cli_path_creation_uses_legacy_migration_key() {
        let cli_path = temp_path("first-create.exe");
        fs::write(&cli_path, "cli").expect("cli fixture");

        let applied = apply_changes(
            &json!({}),
            &[change("claude", json!(cli_path.display().to_string()))],
            &host(Some("WORKSTATION")),
        )
        .expect("apply adapter change");

        assert_eq!(
            applied
                .configuration
                .pointer("/providerConfigs/claude/cliPathsByHost/WORKSTATION"),
            Some(&json!(storable_path(
                &fs_safety::canonical_existing(&cli_path).unwrap()
            )))
        );
        fs::remove_file(cli_path).expect("cleanup");
    }

    #[test]
    fn existing_opaque_device_key_is_updated_without_touching_siblings() {
        let cli_path = temp_path("device-update.exe");
        fs::write(&cli_path, "cli").expect("cli fixture");
        let applied = apply_changes(
            &json!({
                "providerConfigs": {
                    "codex": {
                        "enabled": true,
                        "cliPathsByHost": {"device:only": "C:/old.exe"}
                    }
                }
            }),
            &[change("codex", json!(cli_path.display().to_string()))],
            &host(Some("WORKSTATION")),
        )
        .expect("apply adapter change");

        assert_eq!(
            applied
                .configuration
                .pointer("/providerConfigs/codex/enabled"),
            Some(&json!(true))
        );
        assert_eq!(
            applied
                .configuration
                .pointer("/providerConfigs/codex/cliPathsByHost/device:only"),
            Some(&json!(storable_path(
                &fs_safety::canonical_existing(&cli_path).unwrap()
            )))
        );
        fs::remove_file(cli_path).expect("cleanup");
    }

    #[test]
    fn empty_value_deletes_only_the_selected_device_entry() {
        let applied = apply_changes(
            &json!({
                "providerConfigs": {
                    "pi": {
                        "enabled": true,
                        "cliPathsByHost": {"device:only": "C:/pi.exe"}
                    }
                }
            }),
            &[change("pi", json!(""))],
            &host(Some("WORKSTATION")),
        )
        .expect("delete adapter value");

        assert!(applied
            .configuration
            .pointer("/providerConfigs/pi/cliPathsByHost/device:only")
            .is_none());
        assert_eq!(
            applied.configuration.pointer("/providerConfigs/pi/enabled"),
            Some(&json!(true))
        );
    }

    #[test]
    fn directories_are_rejected_as_cli_paths() {
        let directory = temp_path("directory");
        fs::create_dir_all(&directory).expect("directory fixture");
        let error = apply_changes(
            &json!({}),
            &[change("opencode", json!(directory.display().to_string()))],
            &host(Some("WORKSTATION")),
        )
        .expect_err("directory must be rejected");

        assert_eq!(error.code, "plugin_adapter_path_not_file");
        fs::remove_dir_all(directory).expect("cleanup");
    }

    #[cfg(windows)]
    #[test]
    fn linked_cli_files_are_rejected_when_fixture_is_supported() {
        use std::os::windows::fs::symlink_file;

        let target = temp_path("link-target.exe");
        let link = temp_path("link.exe");
        fs::write(&target, "cli").expect("target fixture");
        if symlink_file(&target, &link).is_err() {
            fs::remove_file(target).expect("cleanup target");
            return;
        }
        let error = apply_changes(
            &json!({}),
            &[change("claude", json!(link.display().to_string()))],
            &host(Some("WORKSTATION")),
        )
        .expect_err("link must be rejected");

        assert_eq!(error.code, "unsupported_link_path");
        fs::remove_file(link).expect("cleanup link");
        fs::remove_file(target).expect("cleanup target");
    }

    #[test]
    fn multiple_opaque_keys_block_writes() {
        let error = apply_changes(
            &json!({
                "providerConfigs": {
                    "claude": {"cliPathsByHost": {
                        "device:first": "C:/one.exe",
                        "device:second": "C:/two.exe"
                    }}
                }
            }),
            &[change("claude", json!(""))],
            &host(Some("WORKSTATION")),
        )
        .expect_err("ambiguous device must be rejected");

        assert_eq!(error.code, "plugin_adapter_device_ambiguous");
    }
}
