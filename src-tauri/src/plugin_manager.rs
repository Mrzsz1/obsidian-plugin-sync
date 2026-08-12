use crate::{
    backup::{timestamp, BackupSession, PluginBackupContext},
    errors::{AppError, AppResult},
    fs_safety,
    models::{
        LocalPluginInstallPreview, ManagedPluginItem, OperationResult, OperationStatus,
        PluginAdapterSettingChange, PluginInventoryItem, PluginSettingsSchema, SyncSummary,
        VaultPluginManagementInventory,
    },
    obsidian_config,
    plugin_adapters::apply_plugin_adapter_changes,
    process::obsidian_is_running,
    reports::write_sync_reports,
    vault::{scan_vault_inventory, validate_vault},
};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

pub fn scan_plugin_management_inventory(
    vault_path: impl Into<String>,
) -> AppResult<VaultPluginManagementInventory> {
    let inventory = scan_vault_inventory(vault_path)?;
    let vault_root = PathBuf::from(&inventory.vault.path);
    let mut plugins = Vec::with_capacity(inventory.plugins.len());

    for plugin in inventory.plugins {
        let (configuration, configuration_error) = if plugin.valid && plugin.has_data_json {
            let plugin_dir = PathBuf::from(&plugin.folder_path);
            fs_safety::ensure_child_path(&vault_root, &plugin_dir)?;
            read_configuration(plugin_dir.join("data.json"))
        } else {
            (None, None)
        };

        plugins.push(ManagedPluginItem {
            plugin,
            configuration,
            configuration_error,
        });
    }

    Ok(VaultPluginManagementInventory {
        vault: inventory.vault,
        plugins,
        warnings: inventory.warnings,
    })
}

fn read_configuration(path: PathBuf) -> (Option<Value>, Option<String>) {
    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) => return (None, Some(format!("无法读取 data.json：{error}"))),
    };

    match serde_json::from_str(&content) {
        Ok(value) => (Some(value), None),
        Err(error) => (None, Some(format!("data.json 不是有效 JSON：{error}"))),
    }
}

pub fn inspect_local_plugin(
    vault_path: String,
    source_folder_path: String,
) -> AppResult<LocalPluginInstallPreview> {
    let vault = validate_vault(vault_path, crate::models::VaultSource::Manual)?;
    inspect_local_plugin_folder(Path::new(&vault.path), Path::new(&source_folder_path))
}

pub fn set_plugin_enabled(
    vault_path: String,
    plugin_id: String,
    enabled: bool,
    obsidian_closed_confirmed: bool,
) -> AppResult<SyncSummary> {
    ensure_write_allowed(obsidian_closed_confirmed)?;
    validate_plugin_id(&plugin_id)?;
    let inventory = scan_vault_inventory(vault_path)?;
    let vault_root = PathBuf::from(&inventory.vault.path);
    let plugin = find_supported_plugin(&inventory.plugins, &plugin_id)?.clone();
    if plugin.enabled == enabled {
        return Err(AppError::new(
            "enabled_state_unchanged",
            "插件启用状态没有变化",
        ));
    }

    let started_at = timestamp();
    let plugin_dir = PathBuf::from(&plugin.folder_path);
    let backup = begin_plugin_backup(
        &vault_root,
        &plugin_id,
        if enabled { "enable" } else { "disable" },
        &plugin_dir,
        plugin.enabled,
    )?;
    let mut enabled_ids: BTreeSet<String> = inventory.enabled_plugin_ids.into_iter().collect();
    if enabled {
        enabled_ids.insert(plugin_id.clone());
    } else {
        enabled_ids.remove(&plugin_id);
    }
    let outcome = obsidian_config::write_enabled_plugin_ids(
        &vault_root,
        &enabled_ids.into_iter().collect::<Vec<_>>(),
    )
    .map(|_| {
        (
            if enabled {
                "已启用插件".to_string()
            } else {
                "已禁用插件，插件文件和设置保持不变".to_string()
            },
            Some(obsidian_config::community_plugins_path(&vault_root)),
        )
    });

    finish_operation(
        started_at,
        &vault_root,
        &plugin_id,
        if enabled {
            "enable-plugin"
        } else {
            "disable-plugin"
        },
        backup,
        outcome,
    )
}

pub fn save_plugin_configuration(
    vault_path: String,
    plugin_id: String,
    configuration: Value,
    obsidian_closed_confirmed: bool,
    risk_override_confirmed: bool,
) -> AppResult<SyncSummary> {
    ensure_write_allowed(obsidian_closed_confirmed)?;
    validate_plugin_id(&plugin_id)?;
    let inventory = scan_vault_inventory(vault_path)?;
    let vault_root = PathBuf::from(&inventory.vault.path);
    let plugin = find_supported_plugin(&inventory.plugins, &plugin_id)?.clone();
    let plugin_dir = PathBuf::from(&plugin.folder_path);
    let data_path = plugin_dir.join("data.json");
    let current = fs::read_to_string(&data_path)
        .ok()
        .and_then(|content| serde_json::from_str::<Value>(&content).ok())
        .unwrap_or_else(|| Value::Object(Default::default()));
    if current == configuration {
        return Err(AppError::new("configuration_unchanged", "插件配置没有变化"));
    }
    let managed_settings = crate::plugin_settings::inspect_plugin_settings(
        vault_root.display().to_string(),
        plugin_id.clone(),
    )?;
    let used_risk_override = validate_configuration_changes(
        &current,
        &configuration,
        &managed_settings.schema,
        risk_override_confirmed,
    )?;

    let started_at = timestamp();
    let backup_operation = if used_risk_override {
        "save-configuration-risk-override"
    } else {
        "save-configuration"
    };
    let result_action = if used_risk_override {
        "save-plugin-configuration-risk-override"
    } else {
        "save-plugin-configuration"
    };
    let backup = begin_plugin_backup(
        &vault_root,
        &plugin_id,
        backup_operation,
        &plugin_dir,
        plugin.enabled,
    )?;
    let success_message = if used_risk_override {
        "已按风险覆盖保存插件配置"
    } else {
        "已保存插件配置"
    };
    let outcome = obsidian_config::write_json_atomic(&data_path, &configuration)
        .map(|_| (success_message.to_string(), Some(data_path)));
    finish_operation(
        started_at,
        &vault_root,
        &plugin_id,
        result_action,
        backup,
        outcome,
    )
}

pub fn save_plugin_adapter_configuration(
    vault_path: String,
    plugin_id: String,
    adapter_id: String,
    changes: Vec<PluginAdapterSettingChange>,
    obsidian_closed_confirmed: bool,
) -> AppResult<SyncSummary> {
    ensure_write_allowed(obsidian_closed_confirmed)?;
    save_plugin_adapter_configuration_after_gate(vault_path, plugin_id, adapter_id, changes)
}

fn save_plugin_adapter_configuration_after_gate(
    vault_path: String,
    plugin_id: String,
    adapter_id: String,
    changes: Vec<PluginAdapterSettingChange>,
) -> AppResult<SyncSummary> {
    validate_plugin_id(&plugin_id)?;
    let inventory = scan_vault_inventory(vault_path)?;
    let vault_root = PathBuf::from(&inventory.vault.path);
    let plugin = find_supported_plugin(&inventory.plugins, &plugin_id)?.clone();
    let plugin_dir = PathBuf::from(&plugin.folder_path);
    let data_path = plugin_dir.join("data.json");
    fs_safety::ensure_child_path(&vault_root, &data_path)?;
    if fs_safety::is_link_path(&data_path)? {
        return Err(
            AppError::new("unsupported_link_path", "不支持修改链接形式的 data.json")
                .with_path(data_path),
        );
    }
    let current = if data_path.exists() {
        let content = fs::read_to_string(&data_path)
            .map_err(|error| AppError::from(error).with_path(&data_path))?;
        serde_json::from_str::<Value>(&content).map_err(|error| {
            AppError::new(
                "plugin_configuration_invalid",
                "data.json 不是有效 JSON，适配器不会覆盖原文件",
            )
            .with_path(&data_path)
            .with_details(error.to_string())
        })?
    } else {
        Value::Object(Default::default())
    };
    let applied = apply_plugin_adapter_changes(&plugin, &current, &adapter_id, &changes)?;
    if current == applied.configuration {
        return Err(AppError::new("configuration_unchanged", "适配设置没有变化"));
    }

    let started_at = timestamp();
    let backup = begin_plugin_backup(
        &vault_root,
        &plugin_id,
        "save-adapter-configuration",
        &plugin_dir,
        plugin.enabled,
    )?;
    let changed_count = applied.changed_fields.len();
    let outcome =
        obsidian_config::write_json_atomic(&data_path, &applied.configuration).map(|_| {
            (
                format!("已通过受信任适配器保存 {changed_count} 项设备本地设置"),
                Some(data_path),
            )
        });
    finish_operation(
        started_at,
        &vault_root,
        &plugin_id,
        "save-plugin-adapter-configuration",
        backup,
        outcome,
    )
}

fn validate_configuration_changes(
    current: &Value,
    next: &Value,
    schema: &PluginSettingsSchema,
    risk_override_confirmed: bool,
) -> AppResult<bool> {
    let mut safe_paths = BTreeSet::new();
    let mut risk_paths = BTreeSet::new();
    for field in schema.groups.iter().flat_map(|group| &group.fields) {
        for path in field
            .path
            .iter()
            .chain(field.path_options.iter().map(|option| &option.path))
        {
            if field.read_only {
                risk_paths.insert(path.clone());
            } else {
                safe_paths.insert(path.clone());
            }
        }
    }

    let mut changed_paths = BTreeSet::new();
    collect_changed_json_paths(Some(current), Some(next), "", &mut changed_paths);
    let mut risky_changes = Vec::new();
    let mut unmapped_changes = Vec::new();
    for changed_path in changed_paths {
        if safe_paths
            .iter()
            .any(|field_path| path_covers_change(field_path, &changed_path))
        {
            continue;
        }
        if risk_paths
            .iter()
            .any(|field_path| path_covers_change(field_path, &changed_path))
        {
            risky_changes.push(changed_path);
        } else {
            unmapped_changes.push(changed_path);
        }
    }

    if !unmapped_changes.is_empty() {
        return Err(AppError::new(
            "unmapped_configuration_change",
            "配置包含未映射字段变更，风险模式也不能写入",
        )
        .with_details(display_changed_paths(&unmapped_changes)));
    }
    if !risky_changes.is_empty() && !risk_override_confirmed {
        return Err(AppError::new(
            "risk_override_required",
            "这些设置包含插件转换或未知保存逻辑，请先确认允许风险编辑",
        )
        .with_details(display_changed_paths(&risky_changes)));
    }
    Ok(!risky_changes.is_empty())
}

fn collect_changed_json_paths(
    current: Option<&Value>,
    next: Option<&Value>,
    path: &str,
    output: &mut BTreeSet<String>,
) {
    if current == next {
        return;
    }
    match (current, next) {
        (Some(Value::Object(current)), Some(Value::Object(next))) => {
            let keys = current
                .keys()
                .chain(next.keys())
                .cloned()
                .collect::<BTreeSet<_>>();
            for key in keys {
                let child_path = append_json_pointer(path, &key);
                collect_changed_json_paths(current.get(&key), next.get(&key), &child_path, output);
            }
        }
        (Some(Value::Object(current)), _) => {
            for (key, value) in current {
                let child_path = append_json_pointer(path, key);
                collect_changed_json_paths(Some(value), None, &child_path, output);
            }
        }
        (_, Some(Value::Object(next))) => {
            for (key, value) in next {
                let child_path = append_json_pointer(path, key);
                collect_changed_json_paths(None, Some(value), &child_path, output);
            }
        }
        _ => {
            output.insert(path.to_string());
        }
    }
}

fn append_json_pointer(path: &str, segment: &str) -> String {
    format!("{path}/{}", segment.replace('~', "~0").replace('/', "~1"))
}

fn path_covers_change(field_path: &str, changed_path: &str) -> bool {
    changed_path == field_path
        || changed_path
            .strip_prefix(field_path)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn display_changed_paths(paths: &[String]) -> String {
    paths
        .iter()
        .map(|path| {
            if path.is_empty() {
                "/".to_string()
            } else {
                path.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn install_plugin_from_local_folder(
    vault_path: String,
    source_folder_path: String,
    overwrite_existing: bool,
    obsidian_closed_confirmed: bool,
) -> AppResult<SyncSummary> {
    ensure_write_allowed(obsidian_closed_confirmed)?;
    let vault = validate_vault(vault_path, crate::models::VaultSource::Manual)?;
    let vault_root = PathBuf::from(&vault.path);
    let preview = inspect_local_plugin_folder(&vault_root, Path::new(&source_folder_path))?;
    if preview.will_overwrite && !overwrite_existing {
        return Err(AppError::new(
            "plugin_overwrite_requires_confirmation",
            "目标知识库已存在同 ID 插件，需要明确确认后才能覆盖",
        ));
    }

    let inventory = scan_vault_inventory(vault.path)?;
    let existing = inventory
        .plugins
        .iter()
        .find(|plugin| plugin.id.as_deref() == Some(&preview.plugin_id));
    let target_dir = existing
        .map(|plugin| PathBuf::from(&plugin.folder_path))
        .unwrap_or_else(|| obsidian_config::plugins_dir(&vault_root).join(&preview.plugin_id));
    fs_safety::ensure_child_path(&vault_root, &target_dir)?;
    if fs_safety::is_link_path(&target_dir)? {
        return Err(
            AppError::new("unsupported_link_path", "不支持覆盖链接目录插件").with_path(&target_dir),
        );
    }
    if target_dir.exists() && existing.is_none() {
        return Err(
            AppError::new("plugin_target_conflict", "目标插件目录已存在但无法安全识别")
                .with_path(&target_dir),
        );
    }

    let started_at = timestamp();
    let enabled_before = existing.is_some_and(|plugin| plugin.enabled);
    let backup = begin_plugin_backup(
        &vault_root,
        &preview.plugin_id,
        if preview.will_overwrite {
            "overwrite-install"
        } else {
            "install"
        },
        &target_dir,
        enabled_before,
    )?;
    let source_dir = PathBuf::from(&preview.source_folder_path);
    let outcome = install_plugin_directory(&source_dir, &target_dir, &vault_root).map(|_| {
        (
            if preview.will_overwrite {
                "已备份并覆盖安装本地插件".to_string()
            } else {
                "已安装本地插件，默认保持禁用".to_string()
            },
            Some(target_dir),
        )
    });
    finish_operation(
        started_at,
        &vault_root,
        &preview.plugin_id,
        if preview.will_overwrite {
            "overwrite-plugin"
        } else {
            "install-plugin"
        },
        backup,
        outcome,
    )
}

pub fn delete_plugin(
    vault_path: String,
    plugin_id: String,
    delete_confirmed: bool,
    secondary_confirmed: bool,
    obsidian_closed_confirmed: bool,
) -> AppResult<SyncSummary> {
    ensure_write_allowed(obsidian_closed_confirmed)?;
    if !delete_confirmed || !secondary_confirmed {
        return Err(AppError::new(
            "plugin_delete_requires_confirmation",
            "删除插件需要明确勾选并进行二次确认",
        ));
    }
    validate_plugin_id(&plugin_id)?;
    let inventory = scan_vault_inventory(vault_path)?;
    let vault_root = PathBuf::from(&inventory.vault.path);
    let plugin = find_supported_plugin(&inventory.plugins, &plugin_id)?.clone();
    let plugin_dir = PathBuf::from(&plugin.folder_path);
    let started_at = timestamp();
    let backup = begin_plugin_backup(
        &vault_root,
        &plugin_id,
        "delete",
        &plugin_dir,
        plugin.enabled,
    )?;
    let outcome = (|| -> AppResult<(String, Option<PathBuf>)> {
        fs_safety::remove_path(&plugin_dir)?;
        let mut enabled_ids: BTreeSet<String> = inventory.enabled_plugin_ids.into_iter().collect();
        enabled_ids.remove(&plugin_id);
        obsidian_config::write_enabled_plugin_ids(
            &vault_root,
            &enabled_ids.into_iter().collect::<Vec<_>>(),
        )?;
        Ok((
            "已删除插件并移除启用状态，可从备份恢复".to_string(),
            Some(plugin_dir),
        ))
    })();
    finish_operation(
        started_at,
        &vault_root,
        &plugin_id,
        "delete-plugin",
        backup,
        outcome,
    )
}

pub fn open_plugin_folder(vault_path: String, plugin_id: String) -> AppResult<()> {
    validate_plugin_id(&plugin_id)?;
    let inventory = scan_vault_inventory(vault_path)?;
    let vault_root = PathBuf::from(&inventory.vault.path);
    let plugin = find_supported_plugin(&inventory.plugins, &plugin_id)?;
    let plugin_dir = PathBuf::from(&plugin.folder_path);
    fs_safety::ensure_child_path(&vault_root, &plugin_dir)?;
    if fs_safety::is_link_path(&plugin_dir)? {
        return Err(
            AppError::new("unsupported_link_path", "不支持打开链接目录插件").with_path(plugin_dir),
        );
    }

    #[cfg(windows)]
    {
        std::process::Command::new("explorer.exe")
            .arg(&plugin_dir)
            .spawn()
            .map_err(|error| AppError::from(error).with_path(&plugin_dir))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        Err(AppError::new(
            "unsupported_platform",
            "打开插件目录目前仅支持 Windows",
        ))
    }
}

fn inspect_local_plugin_folder(
    vault_root: &Path,
    source_folder: &Path,
) -> AppResult<LocalPluginInstallPreview> {
    let source_dir = fs_safety::canonical_existing(source_folder)?;
    if !source_dir.is_dir() {
        return Err(
            AppError::new("invalid_plugin_folder", "请选择一个插件文件夹").with_path(&source_dir),
        );
    }
    if fs_safety::is_link_path(&source_dir)? {
        return Err(
            AppError::new("unsupported_link_path", "不支持链接目录作为插件安装源")
                .with_path(&source_dir),
        );
    }
    let manifest_path = source_dir.join("manifest.json");
    let main_path = source_dir.join("main.js");
    if !manifest_path.is_file() || !main_path.is_file() {
        return Err(AppError::new(
            "incomplete_plugin_folder",
            "插件文件夹必须同时包含 manifest.json 和 main.js",
        )
        .with_path(&source_dir));
    }
    let manifest_content = fs::read_to_string(&manifest_path)
        .map_err(|error| AppError::from(error).with_path(&manifest_path))?;
    let manifest: Value = serde_json::from_str(&manifest_content)
        .map_err(|error| AppError::from(error).with_path(&manifest_path))?;
    let plugin_id = manifest
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            AppError::new("missing_plugin_id", "manifest.json 缺少插件 ID")
                .with_path(&manifest_path)
        })?
        .to_string();
    validate_plugin_id(&plugin_id)?;
    let name = manifest
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or(&plugin_id)
        .to_string();
    let incoming_version = manifest
        .get("version")
        .and_then(Value::as_str)
        .map(str::to_string);
    let inventory = scan_vault_inventory(vault_root.display().to_string())?;
    let existing = inventory
        .plugins
        .iter()
        .find(|plugin| plugin.id.as_deref() == Some(&plugin_id));

    Ok(LocalPluginInstallPreview {
        plugin_id,
        name,
        incoming_version,
        existing_version: existing.and_then(|plugin| plugin.version.clone()),
        source_folder_path: source_dir.display().to_string(),
        will_overwrite: existing.is_some(),
    })
}

pub(crate) fn find_supported_plugin<'a>(
    plugins: &'a [PluginInventoryItem],
    plugin_id: &str,
) -> AppResult<&'a PluginInventoryItem> {
    let plugin = plugins
        .iter()
        .find(|plugin| plugin.id.as_deref() == Some(plugin_id))
        .ok_or_else(|| AppError::new("missing_plugin", "知识库中不存在该插件"))?;
    if !plugin.valid || plugin.unsupported_reason.is_some() {
        return Err(AppError::new(
            "unsupported_plugin",
            "插件无效或属于不支持的链接目录",
        ));
    }
    Ok(plugin)
}

pub(crate) fn validate_plugin_id(plugin_id: &str) -> AppResult<()> {
    let trimmed = plugin_id.trim();
    if trimmed.is_empty()
        || trimmed == "."
        || trimmed == ".."
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.chars().any(char::is_control)
    {
        return Err(AppError::new("invalid_plugin_id", "插件 ID 不安全或无效"));
    }
    Ok(())
}

pub(crate) fn ensure_write_allowed(obsidian_closed_confirmed: bool) -> AppResult<()> {
    if !obsidian_closed_confirmed {
        return Err(AppError::new(
            "obsidian_not_confirmed_closed",
            "请先确认 Obsidian 已关闭",
        ));
    }
    if obsidian_is_running()? {
        return Err(AppError::new(
            "obsidian_running",
            "检测到 Obsidian.exe 正在运行，请关闭后再保存",
        ));
    }
    Ok(())
}

pub(crate) fn begin_plugin_backup(
    vault_root: &Path,
    plugin_id: &str,
    operation: &str,
    plugin_dir: &Path,
    enabled_before: bool,
) -> AppResult<BackupSession> {
    fs_safety::ensure_child_path(vault_root, plugin_dir)?;
    if fs_safety::is_link_path(plugin_dir)? {
        return Err(
            AppError::new("unsupported_link_path", "不支持修改链接目录插件").with_path(plugin_dir),
        );
    }
    let mut backup = BackupSession::create(vault_root, "plugin-management")?;
    backup.set_plugin_context(PluginBackupContext {
        plugin_id: plugin_id.to_string(),
        operation: operation.to_string(),
        enabled_before,
        plugin_directory: plugin_dir.display().to_string(),
    })?;
    backup.backup_path(plugin_dir, "plugin-directory")?;
    backup.backup_path(
        &obsidian_config::community_plugins_path(vault_root),
        "enabled-state-safety-snapshot",
    )?;
    let app_json = obsidian_config::app_json_path(vault_root);
    backup.backup_path(&app_json, "app-json-safety-snapshot")?;
    obsidian_config::ensure_backup_dir_ignored(vault_root)?;
    Ok(backup)
}

fn install_plugin_directory(source: &Path, target: &Path, vault_root: &Path) -> AppResult<()> {
    let plugins_dir = obsidian_config::plugins_dir(vault_root);
    fs::create_dir_all(&plugins_dir)
        .map_err(|error| AppError::from(error).with_path(&plugins_dir))?;
    let stage_dir = plugins_dir.join(format!(".ops-install-{}", timestamp()));
    if stage_dir.exists() {
        fs_safety::remove_path(&stage_dir)?;
    }
    if let Err(error) = fs_safety::copy_path_recursive(source, &stage_dir) {
        let _ = fs_safety::remove_path(&stage_dir);
        return Err(error);
    }

    let target_data = target.join("data.json");
    if target_data.is_file() {
        let stage_data = stage_dir.join("data.json");
        fs::copy(&target_data, &stage_data)
            .map_err(|error| AppError::from(error).with_path(&stage_data))?;
    }
    fs_safety::replace_dir_with_stage(&stage_dir, target)
}

pub(crate) fn finish_operation(
    started_at: String,
    vault_root: &Path,
    plugin_id: &str,
    action: &str,
    backup: BackupSession,
    outcome: AppResult<(String, Option<PathBuf>)>,
) -> AppResult<SyncSummary> {
    let result = match outcome {
        Ok((message, path)) => OperationResult {
            plugin_id: Some(plugin_id.to_string()),
            target_vault_path: vault_root.display().to_string(),
            action: action.to_string(),
            status: OperationStatus::Success,
            message,
            path: path.map(|path| path.display().to_string()),
        },
        Err(error) => OperationResult {
            plugin_id: Some(plugin_id.to_string()),
            target_vault_path: vault_root.display().to_string(),
            action: action.to_string(),
            status: OperationStatus::Failed,
            message: error.message,
            path: error.path,
        },
    };
    let summary = SyncSummary {
        started_at,
        finished_at: timestamp(),
        app_version: crate::models::current_app_version(),
        source_vault_path: None,
        target_vault_paths: vec![vault_root.display().to_string()],
        backup_paths: vec![backup.path().display().to_string()],
        results: vec![result],
    };
    write_sync_reports(
        backup.path(),
        &summary,
        "sync-report.json",
        "sync-report.md",
    )?;
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::restore_backup_dir;
    use crate::models::{
        PluginSettingConfidence, PluginSettingControl, PluginSettingField, PluginSettingGroup,
        PluginSettingSource, PluginSettingSupport, PluginSettingsCompleteness,
        PluginSettingsCoverage, PluginSettingsSchemaSource,
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
        let path = env::temp_dir().join(format!("obsidian-plugin-manager-{name}-{unique}"));
        fs::create_dir_all(path.join(".obsidian/plugins/example")).expect("fixture dirs");
        path
    }

    fn write_manifest(vault: &std::path::Path) {
        fs::write(
            vault.join(".obsidian/plugins/example/manifest.json"),
            r#"{"id":"example","name":"Example","version":"1.2.3"}"#,
        )
        .expect("manifest");
        fs::write(
            vault.join(".obsidian/community-plugins.json"),
            r#"["example"]"#,
        )
        .expect("enabled plugins");
    }

    fn schema_with_paths(safe: &[&str], risky: &[&str]) -> PluginSettingsSchema {
        let fields = safe
            .iter()
            .map(|path| test_field(path, false))
            .chain(risky.iter().map(|path| test_field(path, true)))
            .collect();
        schema_with_fields(fields)
    }

    fn schema_with_fields(fields: Vec<PluginSettingField>) -> PluginSettingsSchema {
        PluginSettingsSchema {
            source: PluginSettingsSchemaSource::Imperative,
            completeness: PluginSettingsCompleteness::Partial,
            coverage: PluginSettingsCoverage::default(),
            groups: vec![PluginSettingGroup {
                id: "test".to_string(),
                title: None,
                page_path: Vec::new(),
                fields,
            }],
            warnings: Vec::new(),
        }
    }

    fn test_field(path: &str, read_only: bool) -> PluginSettingField {
        PluginSettingField {
            id: path.to_string(),
            path: Some(path.to_string()),
            read_paths: vec![path.to_string()],
            path_options: Vec::new(),
            name: path.to_string(),
            description: None,
            control: PluginSettingControl::Text,
            options: Vec::new(),
            placeholder: None,
            min: None,
            max: None,
            step: None,
            default_value: None,
            source: PluginSettingSource::Imperative,
            confidence: PluginSettingConfidence::Exact,
            support: if read_only {
                PluginSettingSupport::RiskTransform
            } else {
                PluginSettingSupport::SafeWritable
            },
            read_only,
            warnings: Vec::new(),
        }
    }

    fn dynamic_test_field(path: &str, read_only: bool) -> PluginSettingField {
        let mut field = test_field("/unused", read_only);
        field.path = None;
        field.read_paths.clear();
        field.path_options = vec![crate::models::PluginSettingPathOption {
            path: path.to_string(),
            label: "existing key".to_string(),
            detail: "existing key".to_string(),
        }];
        field.support = PluginSettingSupport::DynamicExistingKey;
        field
    }

    #[test]
    fn scans_nested_configuration_and_enabled_state() {
        let vault = temp_vault("nested-config");
        write_manifest(&vault);
        fs::write(
            vault.join(".obsidian/plugins/example/data.json"),
            r#"{"enabled":true,"nested":{"count":3},"items":["a",2]}"#,
        )
        .expect("data");

        let inventory =
            scan_plugin_management_inventory(vault.display().to_string()).expect("scan");
        let managed = inventory.plugins.first().expect("managed plugin");
        assert!(managed.plugin.enabled);
        assert_eq!(managed.plugin.version.as_deref(), Some("1.2.3"));
        assert_eq!(
            managed
                .configuration
                .as_ref()
                .and_then(|value| value.pointer("/nested/count")),
            Some(&Value::from(3))
        );
        assert!(managed.configuration_error.is_none());

        fs::remove_dir_all(vault).expect("cleanup");
    }

    #[test]
    fn reports_malformed_configuration_without_failing_inventory() {
        let vault = temp_vault("bad-config");
        write_manifest(&vault);
        fs::write(vault.join(".obsidian/plugins/example/data.json"), "{broken").expect("data");

        let inventory =
            scan_plugin_management_inventory(vault.display().to_string()).expect("scan");
        let managed = inventory.plugins.first().expect("managed plugin");
        assert!(managed.configuration.is_none());
        assert!(managed
            .configuration_error
            .as_deref()
            .is_some_and(|message| message.contains("data.json 不是有效 JSON")));

        fs::remove_dir_all(vault).expect("cleanup");
    }

    #[test]
    fn rejects_incomplete_local_plugin_folder() {
        let vault = temp_vault("incomplete-local");
        write_manifest(&vault);
        let source = vault.parent().expect("parent").join(format!(
            "incomplete-plugin-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&source).expect("source dir");
        fs::write(
            source.join("manifest.json"),
            r#"{"id":"local-example","name":"Local Example","version":"2.0.0"}"#,
        )
        .expect("source manifest");

        let error = inspect_local_plugin(vault.display().to_string(), source.display().to_string())
            .expect_err("missing main.js must be rejected");
        assert_eq!(error.code, "incomplete_plugin_folder");

        fs::remove_dir_all(vault).expect("cleanup vault");
        fs::remove_dir_all(source).expect("cleanup source");
    }

    #[test]
    fn local_install_preserves_existing_configuration() {
        let vault = temp_vault("preserve-install-data");
        write_manifest(&vault);
        let target = vault.join(".obsidian/plugins/example");
        fs::write(target.join("main.js"), "old").expect("old main");
        fs::write(target.join("data.json"), r#"{"keep":true}"#).expect("old data");
        let source = vault.parent().expect("parent").join(format!(
            "install-source-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&source).expect("source dir");
        fs::write(source.join("main.js"), "new").expect("new main");
        fs::write(
            source.join("manifest.json"),
            r#"{"id":"example","name":"Example","version":"2.0.0"}"#,
        )
        .expect("new manifest");
        fs::write(source.join("data.json"), r#"{"replace":true}"#).expect("new data");

        install_plugin_directory(&source, &target, &vault).expect("install");
        assert_eq!(fs::read_to_string(target.join("main.js")).unwrap(), "new");
        assert_eq!(
            fs::read_to_string(target.join("data.json")).unwrap(),
            r#"{"keep":true}"#
        );

        fs::remove_dir_all(vault).expect("cleanup vault");
        fs::remove_dir_all(source).expect("cleanup source");
    }

    #[test]
    fn write_is_rejected_before_process_check_without_confirmation() {
        let error = set_plugin_enabled("unused".into(), "example".into(), true, false)
            .expect_err("confirmation is mandatory");
        assert_eq!(error.code, "obsidian_not_confirmed_closed");
    }

    #[test]
    fn safe_configuration_path_does_not_require_risk_override() {
        let schema = schema_with_paths(&["/enabled"], &["/token"]);
        let used_risk = validate_configuration_changes(
            &json!({"enabled": false, "token": "old"}),
            &json!({"enabled": true, "token": "old"}),
            &schema,
            false,
        )
        .expect("safe path");

        assert!(!used_risk);
    }

    #[test]
    fn known_read_only_path_requires_explicit_risk_override() {
        let schema = schema_with_paths(&["/enabled"], &["/token"]);
        let current = json!({"enabled": false, "token": "old"});
        let next = json!({"enabled": false, "token": " new "});

        let error = validate_configuration_changes(&current, &next, &schema, false)
            .expect_err("risk confirmation required");
        assert_eq!(error.code, "risk_override_required");
        assert!(
            validate_configuration_changes(&current, &next, &schema, true)
                .expect("confirmed risk path")
        );
    }

    #[test]
    fn risk_override_never_allows_unmapped_paths() {
        let schema = schema_with_paths(&["/enabled"], &["/token"]);
        let error = validate_configuration_changes(
            &json!({"enabled": false, "internal": 1}),
            &json!({"enabled": false, "internal": 2}),
            &schema,
            true,
        )
        .expect_err("unmapped path must remain blocked");

        assert_eq!(error.code, "unmapped_configuration_change");
        assert!(error
            .details
            .as_deref()
            .is_some_and(|value| value.contains("/internal")));
    }

    #[test]
    fn known_object_path_covers_nested_changes_and_new_values() {
        let schema = schema_with_paths(&[], &["/provider"]);
        let used_risk = validate_configuration_changes(
            &json!({}),
            &json!({"provider": {"token": "new", "model": "demo"}}),
            &schema,
            true,
        )
        .expect("known object subtree");

        assert!(used_risk);
    }

    #[test]
    fn enumerated_dynamic_path_requires_risk_and_rejects_other_keys() {
        let schema = schema_with_fields(vec![dynamic_test_field(
            "/providerConfigs/codex/byHost/device:known",
            true,
        )]);
        let current = json!({
            "providerConfigs": {"codex": {"byHost": {"device:known": "native"}}}
        });
        let allowed = json!({
            "providerConfigs": {"codex": {"byHost": {"device:known": "wsl"}}}
        });
        let error = validate_configuration_changes(&current, &allowed, &schema, false)
            .expect_err("dynamic risk path needs confirmation");
        assert_eq!(error.code, "risk_override_required");
        assert!(validate_configuration_changes(&current, &allowed, &schema, true).unwrap());

        let forged = json!({
            "providerConfigs": {"codex": {"byHost": {
                "device:known": "native",
                "device:forged": "wsl"
            }}}
        });
        let error = validate_configuration_changes(&current, &forged, &schema, true)
            .expect_err("unlisted dynamic key must remain blocked");
        assert_eq!(error.code, "unmapped_configuration_change");
    }

    #[test]
    fn adapter_save_is_backed_up_and_restorable() {
        let vault = temp_vault("adapter-backup-restore");
        let plugin_dir = vault.join(".obsidian/plugins/realclaudian");
        fs::create_dir_all(&plugin_dir).expect("plugin dir");
        fs::write(
            plugin_dir.join("manifest.json"),
            r#"{"id":"realclaudian","name":"Claudian","version":"2.0.24"}"#,
        )
        .expect("manifest");
        fs::write(plugin_dir.join("main.js"), "module.exports = {};").expect("main");
        fs::write(
            plugin_dir.join("data.json"),
            r#"{"providerConfigs":{"claude":{"cliPathsByHost":{"device:only":"C:/old.cmd"}}}}"#,
        )
        .expect("data");
        fs::write(
            vault.join(".obsidian/community-plugins.json"),
            r#"["realclaudian"]"#,
        )
        .expect("enabled plugins");
        fs::write(vault.join(".obsidian/app.json"), "{}").expect("app json");
        let cli_path = vault.join("claude-cli.cmd");
        fs::write(&cli_path, "@echo off").expect("cli fixture");

        let summary = save_plugin_adapter_configuration_after_gate(
            vault.display().to_string(),
            "realclaudian".to_string(),
            crate::plugin_adapters::CLAUDIAN_ADAPTER_ID.to_string(),
            vec![PluginAdapterSettingChange {
                field_id: "claudian.cli-path.claude".to_string(),
                value: json!(cli_path.display().to_string()),
            }],
        )
        .expect("adapter save");
        assert!(matches!(
            summary.results[0].status,
            OperationStatus::Success
        ));
        assert_eq!(
            summary.results[0].action,
            "save-plugin-adapter-configuration"
        );
        let saved: Value = serde_json::from_str(
            &fs::read_to_string(plugin_dir.join("data.json")).expect("saved data"),
        )
        .expect("saved json");
        assert_ne!(
            saved.pointer("/providerConfigs/claude/cliPathsByHost/device:only"),
            Some(&json!("C:/old.cmd"))
        );

        let backup_path = summary.backup_paths[0].clone();
        let restored = restore_backup_dir(vault.display().to_string(), backup_path, true)
            .expect("restore adapter backup");
        assert!(matches!(
            restored.results[0].status,
            OperationStatus::Success
        ));
        let restored_data: Value = serde_json::from_str(
            &fs::read_to_string(plugin_dir.join("data.json")).expect("restored data"),
        )
        .expect("restored json");
        assert_eq!(
            restored_data.pointer("/providerConfigs/claude/cliPathsByHost/device:only"),
            Some(&json!("C:/old.cmd"))
        );

        fs::remove_dir_all(vault).expect("cleanup");
    }
}
