use crate::{
    backup::timestamp,
    errors::{AppError, AppResult},
    fs_safety,
    models::{
        PluginInventoryItem, PluginRuntimeSettingsSnapshot, SettingsBridgeInstallationStatus,
        SettingsBridgeRequestOperation, SettingsBridgeSnapshotStatus, SettingsBridgeStatus,
        SyncSummary,
    },
    obsidian_config,
    plugin_manager::{
        begin_plugin_backup, ensure_write_allowed, find_supported_plugin, finish_operation,
        set_plugin_enabled, validate_plugin_id,
    },
    process::obsidian_is_running,
    vault::scan_vault_inventory,
};
use semver::Version;
use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub const BRIDGE_PLUGIN_ID: &str = "obsidian-plugin-sync-bridge";
pub const BRIDGE_VERSION: &str = "0.1.0";
pub const BRIDGE_PROTOCOL_VERSION: u32 = 1;
pub const BRIDGE_CACHE_VERSION: u32 = 1;
const MAX_STATUS_BYTES: u64 = 128 * 1024;
const MAX_SNAPSHOT_BYTES: u64 = 4 * 1024 * 1024;
const BUNDLED_MANIFEST: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../bridge-plugin/manifest.json"
));
const BUNDLED_MAIN: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../bridge-plugin/main.js"
));
const BUNDLED_STYLES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../bridge-plugin/styles.css"
));

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BridgeRuntimeStatusFile {
    bridge_version: String,
    protocol_version: u32,
    cache_version: u32,
    obsidian_version: String,
    locale: String,
    vault_name: String,
    updated_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BridgeFingerprintFile {
    plugin_id: String,
    plugin_version: Option<String>,
    plugin_main_hash: String,
    obsidian_version: String,
    locale: String,
    protocol_version: u32,
    configuration_hash: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BridgeCacheFile {
    cache_version: u32,
    captured_at: String,
    fingerprint: BridgeFingerprintFile,
    snapshot: PluginRuntimeSettingsSnapshot,
}

#[derive(Debug)]
pub struct SettingsBridgeInspection {
    pub status: SettingsBridgeStatus,
    pub snapshot: Option<PluginRuntimeSettingsSnapshot>,
}

pub fn inspect_settings_bridge(
    vault_path: String,
    plugin_id: String,
) -> AppResult<SettingsBridgeStatus> {
    validate_plugin_id(&plugin_id)?;
    let inventory = scan_vault_inventory(vault_path)?;
    let plugin = find_supported_plugin(&inventory.plugins, &plugin_id)?;
    Ok(inspect_bridge_for_plugin(Path::new(&inventory.vault.path), plugin).status)
}

pub fn inspect_bridge_for_plugin(
    vault_root: &Path,
    plugin: &PluginInventoryItem,
) -> SettingsBridgeInspection {
    let mut status = empty_status(plugin.id.as_deref().unwrap_or(&plugin.folder_name));
    let bridge_dir = obsidian_config::plugins_dir(vault_root).join(BRIDGE_PLUGIN_ID);
    if !bridge_dir.exists() {
        return SettingsBridgeInspection {
            status,
            snapshot: None,
        };
    }
    if fs_safety::ensure_child_path(vault_root, &bridge_dir).is_err()
        || fs_safety::is_link_path(&bridge_dir).unwrap_or(true)
    {
        status.installation = SettingsBridgeInstallationStatus::Invalid;
        status
            .warnings
            .push("Bridge 目录是链接、重解析点或不在当前知识库内".to_string());
        status.snapshot = SettingsBridgeSnapshotStatus::Invalid;
        return SettingsBridgeInspection {
            status,
            snapshot: None,
        };
    }

    let manifest_path = bridge_dir.join("manifest.json");
    let main_path = bridge_dir.join("main.js");
    let installed = read_bridge_manifest(&manifest_path);
    let installed = match installed {
        Ok(installed) => installed,
        Err(error) => {
            status.installation = SettingsBridgeInstallationStatus::Invalid;
            status.snapshot = SettingsBridgeSnapshotStatus::Invalid;
            status.warnings.push(error.message);
            return SettingsBridgeInspection {
                status,
                snapshot: None,
            };
        }
    };
    status.installed_version = Some(installed.version.clone());
    if installed.id != BRIDGE_PLUGIN_ID || !main_path.is_file() {
        status.installation = SettingsBridgeInstallationStatus::Invalid;
        status.snapshot = SettingsBridgeSnapshotStatus::Invalid;
        status
            .warnings
            .push("Bridge 插件文件不完整或 manifest ID 不匹配".to_string());
        return SettingsBridgeInspection {
            status,
            snapshot: None,
        };
    }
    if fs_safety::is_link_path(&main_path).unwrap_or(true) {
        status.installation = SettingsBridgeInstallationStatus::Invalid;
        status.snapshot = SettingsBridgeSnapshotStatus::Invalid;
        status
            .warnings
            .push("Bridge main.js 不支持链接文件".to_string());
        return SettingsBridgeInspection {
            status,
            snapshot: None,
        };
    }
    status.enabled = obsidian_config::read_enabled_plugin_ids(vault_root)
        .map(|ids| ids.iter().any(|id| id == BRIDGE_PLUGIN_ID))
        .unwrap_or(false);
    status.installation = if installed.version != BRIDGE_VERSION {
        SettingsBridgeInstallationStatus::VersionMismatch
    } else if !status.enabled {
        SettingsBridgeInstallationStatus::Disabled
    } else {
        SettingsBridgeInstallationStatus::Ready
    };
    if installed.version != BRIDGE_VERSION {
        status.warnings.push(format!(
            "Bridge 版本不匹配：已安装 {}，桌面端内置 {}",
            installed.version, BRIDGE_VERSION
        ));
    }

    let target_id = plugin.id.as_deref().unwrap_or(&plugin.folder_name);
    let cache_path = bridge_dir
        .join("cache")
        .join(format!("v{BRIDGE_CACHE_VERSION}"))
        .join(format!("{target_id}.json"));
    if !cache_path.exists() {
        return SettingsBridgeInspection {
            status,
            snapshot: None,
        };
    }
    if fs_safety::is_link_path(&cache_path).unwrap_or(true) {
        status.snapshot = SettingsBridgeSnapshotStatus::Invalid;
        status
            .warnings
            .push("Bridge 快照不支持链接文件".to_string());
        return SettingsBridgeInspection {
            status,
            snapshot: None,
        };
    }
    let cache = match read_bounded_json::<BridgeCacheFile>(&cache_path, MAX_SNAPSHOT_BYTES) {
        Ok(cache) => cache,
        Err(error) => {
            status.snapshot = SettingsBridgeSnapshotStatus::Invalid;
            status.warnings.push(error.message);
            return SettingsBridgeInspection {
                status,
                snapshot: None,
            };
        }
    };
    status.captured_at = Some(cache.captured_at.clone());
    status.field_count = cache.snapshot.fields.len();

    let runtime_path = bridge_dir.join("bridge-status.json");
    let runtime = read_bounded_json::<BridgeRuntimeStatusFile>(&runtime_path, MAX_STATUS_BYTES);
    let mut stale_reasons = Vec::new();
    let runtime = match runtime {
        Ok(runtime) => Some(runtime),
        Err(error) => {
            stale_reasons.push(format!("无法验证当前 Obsidian 运行环境：{}", error.message));
            None
        }
    };
    if cache.cache_version != BRIDGE_CACHE_VERSION {
        stale_reasons.push("缓存格式版本已变化".to_string());
    }
    if cache.snapshot.protocol_version != BRIDGE_PROTOCOL_VERSION
        || cache.fingerprint.protocol_version != BRIDGE_PROTOCOL_VERSION
    {
        stale_reasons.push("Bridge 协议版本已变化".to_string());
    }
    if cache.snapshot.plugin_id != target_id || cache.fingerprint.plugin_id != target_id {
        stale_reasons.push("快照插件 ID 与当前插件不匹配".to_string());
    }
    if cache.snapshot.plugin_version != plugin.version
        || cache.fingerprint.plugin_version != plugin.version
    {
        stale_reasons.push("插件版本已变化".to_string());
    }
    let plugin_dir = PathBuf::from(&plugin.folder_path);
    let main_hash = file_fingerprint(&plugin_dir.join("main.js"));
    let config_hash = file_fingerprint(&plugin_dir.join("data.json"));
    if cache.fingerprint.plugin_main_hash != main_hash {
        stale_reasons.push("main.js 已变化".to_string());
    }
    if cache.fingerprint.configuration_hash != config_hash {
        stale_reasons.push("data.json 已变化，条件设置或选项可能不同".to_string());
    }
    if let Some(runtime) = runtime {
        if runtime.bridge_version != BRIDGE_VERSION
            || runtime.protocol_version != BRIDGE_PROTOCOL_VERSION
            || runtime.cache_version != BRIDGE_CACHE_VERSION
        {
            stale_reasons.push("Bridge 运行状态版本不匹配".to_string());
        }
        if runtime.vault_name != vault_name(vault_root) {
            stale_reasons.push("Bridge 运行状态属于其他知识库".to_string());
        }
        if runtime.obsidian_version != cache.fingerprint.obsidian_version {
            stale_reasons.push("Obsidian 版本已变化".to_string());
        }
        if runtime.locale != cache.fingerprint.locale {
            stale_reasons.push("Obsidian 语言已变化".to_string());
        }
        if runtime.updated_at.trim().is_empty() {
            stale_reasons.push("Bridge 运行状态缺少更新时间".to_string());
        }
    }
    if !matches!(status.installation, SettingsBridgeInstallationStatus::Ready) {
        stale_reasons.push("Bridge 未处于已启用且兼容状态".to_string());
    }

    if stale_reasons.is_empty() {
        status.snapshot = SettingsBridgeSnapshotStatus::Fresh;
        SettingsBridgeInspection {
            status,
            snapshot: Some(cache.snapshot),
        }
    } else {
        status.snapshot = SettingsBridgeSnapshotStatus::Stale;
        status.warnings.extend(stale_reasons);
        SettingsBridgeInspection {
            status,
            snapshot: None,
        }
    }
}

pub fn install_settings_bridge(
    vault_path: String,
    enable_after_install: bool,
    allow_downgrade: bool,
    obsidian_closed_confirmed: bool,
) -> AppResult<SyncSummary> {
    ensure_write_allowed(obsidian_closed_confirmed)?;
    install_settings_bridge_after_gate(vault_path, enable_after_install, allow_downgrade)
}

fn install_settings_bridge_after_gate(
    vault_path: String,
    enable_after_install: bool,
    allow_downgrade: bool,
) -> AppResult<SyncSummary> {
    let inventory = scan_vault_inventory(vault_path)?;
    let vault_root = PathBuf::from(&inventory.vault.path);
    let bridge_dir = obsidian_config::plugins_dir(&vault_root).join(BRIDGE_PLUGIN_ID);
    let enabled_before = inventory
        .enabled_plugin_ids
        .iter()
        .any(|id| id == BRIDGE_PLUGIN_ID);
    let installed_version = if bridge_dir.join("manifest.json").is_file() {
        match read_bridge_manifest(&bridge_dir.join("manifest.json")) {
            Ok(manifest) => Some(manifest.version),
            Err(_) if allow_downgrade => None,
            Err(error) => {
                return Err(AppError::new(
                    "bridge_repair_confirmation_required",
                    "Bridge manifest 无效，需要在修复确认中允许覆盖",
                )
                .with_details(error.to_string()));
            }
        }
    } else {
        None
    };
    if let Some(version) = installed_version.as_deref() {
        let installed = Version::parse(version).map_err(|error| {
            AppError::new("bridge_version_invalid", "已安装 Bridge 版本无效")
                .with_details(error.to_string())
        })?;
        let bundled = Version::parse(BRIDGE_VERSION).expect("bundled Bridge version is semver");
        if installed > bundled && !allow_downgrade {
            return Err(AppError::new(
                "bridge_downgrade_confirmation_required",
                "已安装 Bridge 比桌面端内置版本更新，默认禁止降级",
            ));
        }
    }
    let operation = if bridge_dir.exists() {
        "update-settings-bridge"
    } else {
        "install-settings-bridge"
    };
    let started_at = timestamp();
    let backup = begin_plugin_backup(
        &vault_root,
        BRIDGE_PLUGIN_ID,
        operation,
        &bridge_dir,
        enabled_before,
    )?;
    let outcome = stage_bridge_install(&vault_root, &bridge_dir).and_then(|_| {
        let mut enabled = obsidian_config::read_enabled_plugin_ids(&vault_root)?;
        if enable_after_install && !enabled.iter().any(|id| id == BRIDGE_PLUGIN_ID) {
            enabled.push(BRIDGE_PLUGIN_ID.to_string());
            obsidian_config::write_enabled_plugin_ids(&vault_root, &enabled)?;
        }
        Ok((
            if installed_version.is_some() {
                "已备份并更新 Obsidian 设置 Bridge"
            } else {
                "已安装 Obsidian 设置 Bridge"
            }
            .to_string(),
            Some(bridge_dir.clone()),
        ))
    });
    finish_operation(
        started_at,
        &vault_root,
        BRIDGE_PLUGIN_ID,
        operation,
        backup,
        outcome,
    )
}

pub fn set_settings_bridge_enabled(
    vault_path: String,
    enabled: bool,
    obsidian_closed_confirmed: bool,
) -> AppResult<SyncSummary> {
    set_plugin_enabled(
        vault_path,
        BRIDGE_PLUGIN_ID.to_string(),
        enabled,
        obsidian_closed_confirmed,
    )
}

pub fn remove_settings_bridge(
    vault_path: String,
    remove_confirmed: bool,
    obsidian_closed_confirmed: bool,
) -> AppResult<SyncSummary> {
    if !remove_confirmed {
        return Err(AppError::new(
            "bridge_remove_confirmation_required",
            "移除 Bridge 需要明确确认",
        ));
    }
    ensure_write_allowed(obsidian_closed_confirmed)?;
    remove_settings_bridge_after_gate(vault_path)
}

fn remove_settings_bridge_after_gate(vault_path: String) -> AppResult<SyncSummary> {
    let inventory = scan_vault_inventory(vault_path)?;
    let vault_root = PathBuf::from(&inventory.vault.path);
    let bridge_dir = obsidian_config::plugins_dir(&vault_root).join(BRIDGE_PLUGIN_ID);
    if !bridge_dir.exists() {
        return Err(AppError::new(
            "bridge_not_installed",
            "当前知识库未安装 Bridge",
        ));
    }
    let enabled_before = inventory
        .enabled_plugin_ids
        .iter()
        .any(|id| id == BRIDGE_PLUGIN_ID);
    let started_at = timestamp();
    let backup = begin_plugin_backup(
        &vault_root,
        BRIDGE_PLUGIN_ID,
        "remove-settings-bridge",
        &bridge_dir,
        enabled_before,
    )?;
    let outcome = (|| -> AppResult<(String, Option<PathBuf>)> {
        let mut enabled = obsidian_config::read_enabled_plugin_ids(&vault_root)?;
        enabled.retain(|id| id != BRIDGE_PLUGIN_ID);
        obsidian_config::write_enabled_plugin_ids(&vault_root, &enabled)?;
        fs_safety::remove_path(&bridge_dir)?;
        Ok((
            "已移除 Bridge；其他插件未被修改，备份仍可恢复".to_string(),
            Some(bridge_dir.clone()),
        ))
    })();
    finish_operation(
        started_at,
        &vault_root,
        BRIDGE_PLUGIN_ID,
        "remove-settings-bridge",
        backup,
        outcome,
    )
}

pub fn launch_settings_bridge_request(
    vault_path: String,
    plugin_id: String,
    operation: SettingsBridgeRequestOperation,
) -> AppResult<()> {
    validate_plugin_id(&plugin_id)?;
    if !obsidian_is_running()? {
        return Err(AppError::new(
            "obsidian_not_running_for_bridge",
            "请先启动 Obsidian，再请求 Bridge 抓取或打开真实设置页",
        ));
    }
    let inventory = scan_vault_inventory(vault_path)?;
    let plugin = find_supported_plugin(&inventory.plugins, &plugin_id)?;
    let inspection = inspect_bridge_for_plugin(Path::new(&inventory.vault.path), plugin);
    if !matches!(
        inspection.status.installation,
        SettingsBridgeInstallationStatus::Ready
    ) {
        return Err(AppError::new(
            "bridge_not_ready",
            "Bridge 未安装、未启用或版本不兼容",
        ));
    }
    let uri = build_bridge_uri(&inventory.vault.name, &plugin_id, &operation);
    Command::new("explorer.exe")
        .arg(&uri)
        .spawn()
        .map_err(|error| {
            AppError::new("bridge_uri_launch_failed", "无法启动 Obsidian Bridge 请求")
                .with_details(error.to_string())
        })?;
    Ok(())
}

fn stage_bridge_install(vault_root: &Path, bridge_dir: &Path) -> AppResult<()> {
    let plugins_dir = obsidian_config::plugins_dir(vault_root);
    fs::create_dir_all(&plugins_dir)
        .map_err(|error| AppError::from(error).with_path(&plugins_dir))?;
    let stage = plugins_dir.join(format!(".ops-bridge-{}", timestamp()));
    fs::create_dir_all(&stage).map_err(|error| AppError::from(error).with_path(&stage))?;
    let result = (|| -> AppResult<()> {
        fs::write(stage.join("manifest.json"), BUNDLED_MANIFEST)
            .map_err(|error| AppError::from(error).with_path(stage.join("manifest.json")))?;
        fs::write(stage.join("main.js"), BUNDLED_MAIN)
            .map_err(|error| AppError::from(error).with_path(stage.join("main.js")))?;
        fs::write(stage.join("styles.css"), BUNDLED_STYLES)
            .map_err(|error| AppError::from(error).with_path(stage.join("styles.css")))?;
        if bridge_dir.exists() {
            for retained in ["cache", "data.json", "bridge-status.json"] {
                let source = bridge_dir.join(retained);
                if source.exists() {
                    fs_safety::copy_path_recursive(&source, &stage.join(retained))?;
                }
            }
        }
        fs_safety::replace_dir_with_stage(&stage, bridge_dir)
    })();
    if result.is_err() && stage.exists() {
        let _ = fs_safety::remove_path(&stage);
    }
    result
}

#[derive(Debug)]
struct BridgeManifest {
    id: String,
    version: String,
}

fn read_bridge_manifest(path: &Path) -> AppResult<BridgeManifest> {
    let content =
        fs::read_to_string(path).map_err(|error| AppError::from(error).with_path(path))?;
    let value: serde_json::Value =
        serde_json::from_str(&content).map_err(|error| AppError::from(error).with_path(path))?;
    let object = value.as_object().ok_or_else(|| {
        AppError::new("bridge_manifest_invalid", "Bridge manifest 根节点不是对象").with_path(path)
    })?;
    let id = object
        .get("id")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| AppError::new("bridge_manifest_invalid", "Bridge manifest 缺少 ID"))?;
    let version = object
        .get("version")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| AppError::new("bridge_manifest_invalid", "Bridge manifest 缺少版本"))?;
    Ok(BridgeManifest {
        id: id.to_string(),
        version: version.to_string(),
    })
}

fn read_bounded_json<T: for<'de> Deserialize<'de>>(path: &Path, max_bytes: u64) -> AppResult<T> {
    let metadata = fs::metadata(path).map_err(|error| AppError::from(error).with_path(path))?;
    if metadata.len() > max_bytes {
        return Err(AppError::new(
            "bridge_cache_too_large",
            "Bridge 状态或快照文件超过安全大小上限",
        )
        .with_path(path));
    }
    let content =
        fs::read_to_string(path).map_err(|error| AppError::from(error).with_path(path))?;
    serde_json::from_str(&content).map_err(|error| AppError::from(error).with_path(path))
}

fn file_fingerprint(path: &Path) -> String {
    if !path.exists() {
        return "missing".to_string();
    }
    if fs_safety::is_link_path(path).unwrap_or(true) {
        return "unsupported-link".to_string();
    }
    match fs::read(path) {
        Ok(bytes) => fnv1a64(&bytes),
        Err(_) => "unreadable".to_string(),
    }
}

fn fnv1a64(bytes: &[u8]) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{}-{hash:016x}", bytes.len())
}

fn build_bridge_uri(
    vault_name: &str,
    plugin_id: &str,
    operation: &SettingsBridgeRequestOperation,
) -> String {
    let operation = match operation {
        SettingsBridgeRequestOperation::Capture => "capture",
        SettingsBridgeRequestOperation::OpenSettings => "open-settings",
    };
    format!(
        "obsidian://{BRIDGE_PLUGIN_ID}?protocol={BRIDGE_PROTOCOL_VERSION}&vault={}&op={operation}&plugin={}",
        percent_encode(vault_name),
        percent_encode(plugin_id),
    )
}

fn percent_encode(value: &str) -> String {
    let mut output = String::new();
    for byte in value.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            output.push(char::from(*byte));
        } else {
            output.push_str(&format!("%{byte:02X}"));
        }
    }
    output
}

fn vault_name(vault_root: &Path) -> String {
    vault_root
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn empty_status(plugin_id: &str) -> SettingsBridgeStatus {
    SettingsBridgeStatus {
        plugin_id: plugin_id.to_string(),
        bridge_id: BRIDGE_PLUGIN_ID.to_string(),
        bundled_version: BRIDGE_VERSION.to_string(),
        installed_version: None,
        installation: SettingsBridgeInstallationStatus::Missing,
        enabled: false,
        protocol_version: BRIDGE_PROTOCOL_VERSION,
        snapshot: SettingsBridgeSnapshotStatus::Missing,
        captured_at: None,
        field_count: 0,
        warnings: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        backup::restore_backup_dir_after_gate,
        models::{OperationStatus, VaultSource},
        vault::validate_vault,
    };
    use serde_json::json;
    use std::{
        env,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn temp_vault(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let vault = env::temp_dir().join(format!("settings-bridge-{name}-{unique}"));
        let plugin = vault.join(".obsidian/plugins/example");
        fs::create_dir_all(&plugin).expect("fixture dirs");
        fs::write(
            plugin.join("manifest.json"),
            r#"{"id":"example","name":"Example","version":"1.2.3"}"#,
        )
        .unwrap();
        fs::write(plugin.join("main.js"), "example source").unwrap();
        fs::write(plugin.join("data.json"), r#"{"mode":"one"}"#).unwrap();
        fs::write(
            vault.join(".obsidian/community-plugins.json"),
            r#"["example"]"#,
        )
        .unwrap();
        vault
    }

    fn target_plugin(vault: &Path) -> PluginInventoryItem {
        scan_vault_inventory(vault.display().to_string())
            .unwrap()
            .plugins
            .into_iter()
            .find(|plugin| plugin.id.as_deref() == Some("example"))
            .unwrap()
    }

    fn write_fresh_cache(vault: &Path) {
        let bridge = vault.join(".obsidian/plugins").join(BRIDGE_PLUGIN_ID);
        fs::create_dir_all(bridge.join("cache/v1")).unwrap();
        fs::write(bridge.join("manifest.json"), BUNDLED_MANIFEST).unwrap();
        fs::write(bridge.join("main.js"), BUNDLED_MAIN).unwrap();
        fs::write(bridge.join("styles.css"), BUNDLED_STYLES).unwrap();
        fs::write(
            bridge.join("bridge-status.json"),
            serde_json::to_vec_pretty(&json!({
                "bridgeVersion": BRIDGE_VERSION,
                "protocolVersion": BRIDGE_PROTOCOL_VERSION,
                "cacheVersion": BRIDGE_CACHE_VERSION,
                "obsidianVersion": "1.13.1",
                "locale": "zh",
                "vaultName": vault.file_name().unwrap().to_string_lossy(),
                "updatedAt": "2026-07-11T00:00:00Z"
            }))
            .unwrap(),
        )
        .unwrap();
        let plugin = target_plugin(vault);
        let plugin_dir = PathBuf::from(&plugin.folder_path);
        fs::write(
            bridge.join("cache/v1/example.json"),
            serde_json::to_vec_pretty(&json!({
                "cacheVersion": BRIDGE_CACHE_VERSION,
                "capturedAt": "2026-07-11T00:00:00Z",
                "fingerprint": {
                    "pluginId": "example",
                    "pluginVersion": "1.2.3",
                    "pluginMainHash": file_fingerprint(&plugin_dir.join("main.js")),
                    "obsidianVersion": "1.13.1",
                    "locale": "zh",
                    "protocolVersion": BRIDGE_PROTOCOL_VERSION,
                    "configurationHash": file_fingerprint(&plugin_dir.join("data.json"))
                },
                "snapshot": {
                    "protocolVersion": BRIDGE_PROTOCOL_VERSION,
                    "pluginId": "example",
                    "pluginVersion": "1.2.3",
                    "fields": [{
                        "pagePath": [], "groupTitle": null, "order": 0,
                        "name": "Runtime model", "description": null,
                        "control": "dropdown", "options": [{"value":"one","label":"One"}],
                        "placeholder": null, "min": null, "max": null, "step": null,
                        "disabled": false, "visible": true, "action": false,
                        "confidence": "exact"
                    }],
                    "warnings": []
                }
            }))
            .unwrap(),
        )
        .unwrap();
        let mut enabled = obsidian_config::read_enabled_plugin_ids(vault).unwrap();
        enabled.push(BRIDGE_PLUGIN_ID.to_string());
        obsidian_config::write_enabled_plugin_ids(vault, &enabled).unwrap();
    }

    #[test]
    fn validates_bundled_manifest_and_uri_shape() {
        let manifest: serde_json::Value = serde_json::from_slice(BUNDLED_MANIFEST).unwrap();
        assert_eq!(manifest["id"], BRIDGE_PLUGIN_ID);
        assert_eq!(manifest["version"], BRIDGE_VERSION);
        assert_eq!(fnv1a64(b"hello"), "5-a430d84680aabd0b");
        assert_eq!(
            build_bridge_uri("Vault 名", "example", &SettingsBridgeRequestOperation::Capture),
            "obsidian://obsidian-plugin-sync-bridge?protocol=1&vault=Vault%20%E5%90%8D&op=capture&plugin=example"
        );
    }

    #[test]
    fn loads_only_a_fresh_matching_snapshot() {
        let vault = temp_vault("fresh");
        write_fresh_cache(&vault);
        let plugin = target_plugin(&vault);
        let inspection = inspect_bridge_for_plugin(&vault, &plugin);
        assert_eq!(
            inspection.status.installation,
            SettingsBridgeInstallationStatus::Ready
        );
        assert_eq!(
            inspection.status.snapshot,
            SettingsBridgeSnapshotStatus::Fresh
        );
        assert_eq!(inspection.snapshot.unwrap().fields.len(), 1);

        fs::write(
            vault.join(".obsidian/plugins/example/data.json"),
            r#"{"mode":"two"}"#,
        )
        .unwrap();
        let stale = inspect_bridge_for_plugin(&vault, &target_plugin(&vault));
        assert_eq!(stale.status.snapshot, SettingsBridgeSnapshotStatus::Stale);
        assert!(stale.snapshot.is_none());

        fs::write(
            vault.join(".obsidian/plugins/example/data.json"),
            r#"{"mode":"one"}"#,
        )
        .unwrap();
        let status_path =
            vault.join(".obsidian/plugins/obsidian-plugin-sync-bridge/bridge-status.json");
        let mut runtime: serde_json::Value =
            serde_json::from_slice(&fs::read(&status_path).unwrap()).unwrap();
        runtime["locale"] = json!("en");
        fs::write(&status_path, serde_json::to_vec_pretty(&runtime).unwrap()).unwrap();
        let locale_stale = inspect_bridge_for_plugin(&vault, &target_plugin(&vault));
        assert_eq!(
            locale_stale.status.snapshot,
            SettingsBridgeSnapshotStatus::Stale
        );
        assert!(locale_stale
            .status
            .warnings
            .iter()
            .any(|warning| warning.contains("语言")));
        fs::remove_dir_all(vault).unwrap();
    }

    #[test]
    fn installs_updates_and_restores_bridge_without_touching_other_plugin() {
        let vault = temp_vault("install");
        validate_vault(vault.display().to_string(), VaultSource::Manual).unwrap();
        let original_main = fs::read(vault.join(".obsidian/plugins/example/main.js")).unwrap();
        let summary = install_settings_bridge_after_gate(vault.display().to_string(), true, false)
            .expect("install Bridge");
        assert!(vault
            .join(".obsidian/plugins/obsidian-plugin-sync-bridge/main.js")
            .is_file());
        assert_eq!(
            fs::read(vault.join(".obsidian/plugins/example/main.js")).unwrap(),
            original_main
        );
        assert!(obsidian_config::read_enabled_plugin_ids(&vault)
            .unwrap()
            .contains(&BRIDGE_PLUGIN_ID.to_string()));

        let removed =
            remove_settings_bridge_after_gate(vault.display().to_string()).expect("remove Bridge");
        assert!(!vault
            .join(".obsidian/plugins/obsidian-plugin-sync-bridge")
            .exists());
        restore_backup_dir_after_gate(vault.display().to_string(), removed.backup_paths[0].clone())
            .unwrap();
        assert!(vault
            .join(".obsidian/plugins/obsidian-plugin-sync-bridge/main.js")
            .is_file());
        assert_eq!(
            fs::read(vault.join(".obsidian/plugins/example/main.js")).unwrap(),
            original_main
        );
        assert!(matches!(
            summary.results[0].status,
            OperationStatus::Success
        ));
        fs::remove_dir_all(vault).unwrap();
    }

    #[test]
    fn damaged_bridge_requires_explicit_repair_and_can_be_reinstalled() {
        let vault = temp_vault("repair");
        let bridge = vault.join(".obsidian/plugins").join(BRIDGE_PLUGIN_ID);
        fs::create_dir_all(&bridge).unwrap();
        fs::write(bridge.join("manifest.json"), "{broken").unwrap();
        fs::write(bridge.join("main.js"), "broken bridge").unwrap();

        let status = inspect_bridge_for_plugin(&vault, &target_plugin(&vault));
        assert_eq!(
            status.status.installation,
            SettingsBridgeInstallationStatus::Invalid
        );
        let error = install_settings_bridge_after_gate(vault.display().to_string(), true, false)
            .expect_err("repair must require explicit overwrite confirmation");
        assert_eq!(error.code, "bridge_repair_confirmation_required");

        install_settings_bridge_after_gate(vault.display().to_string(), true, true)
            .expect("confirmed repair");
        let repaired = read_bridge_manifest(&bridge.join("manifest.json")).unwrap();
        assert_eq!(repaired.id, BRIDGE_PLUGIN_ID);
        assert_eq!(repaired.version, BRIDGE_VERSION);
        assert_eq!(
            fs::read(vault.join(".obsidian/plugins/example/main.js")).unwrap(),
            b"example source"
        );
        fs::remove_dir_all(vault).unwrap();
    }
}
