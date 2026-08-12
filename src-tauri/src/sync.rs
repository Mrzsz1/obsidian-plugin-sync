use crate::{
    backup::{timestamp, BackupSession},
    errors::{AppError, AppResult},
    fs_safety,
    models::{
        OperationResult, OperationStatus, PluginInventoryItem, SelectedPluginOperation, SyncPlan,
        SyncSummary,
    },
    obsidian_config,
    plugin_adapters::{prepare_configuration_for_sync, should_filter_configuration_for_sync},
    process::obsidian_is_running,
    reports::write_sync_reports,
    settings_bridge::BRIDGE_PLUGIN_ID,
    vault::scan_vault_inventory,
    versions::compare_versions,
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

pub fn apply_plan(plan: SyncPlan) -> AppResult<SyncSummary> {
    if !plan.obsidian_closed_confirmed {
        return Err(AppError::new(
            "obsidian_not_confirmed_closed",
            "请先确认 Obsidian 已关闭",
        ));
    }
    if obsidian_is_running()? {
        return Err(AppError::new(
            "obsidian_running",
            "检测到 Obsidian.exe 正在运行，请关闭后再应用",
        ));
    }

    let started_at = timestamp();
    let source_inventory = scan_vault_inventory(plan.source_vault_path.clone())?;
    let source_vault_path = PathBuf::from(&source_inventory.vault.path);
    let mut operations_by_target: BTreeMap<String, Vec<SelectedPluginOperation>> = BTreeMap::new();
    for operation in plan.operations {
        validate_operation_shape(&operation, &plan.source_vault_path)?;
        operations_by_target
            .entry(operation.target_vault_path.clone())
            .or_default()
            .push(operation);
    }

    let mut backup_paths = Vec::new();
    let mut applied_target_paths = Vec::new();
    let mut results = Vec::new();

    for target_path in plan.target_vault_paths {
        let target_operations = operations_by_target
            .remove(&target_path)
            .unwrap_or_default();
        if target_operations.is_empty() {
            continue;
        }
        let target_inventory = scan_vault_inventory(target_path.clone())?;
        let target_vault_path = PathBuf::from(&target_inventory.vault.path);
        let mut backup = BackupSession::create(&target_vault_path, "sync")?;
        backup_paths.push(backup.path().display().to_string());
        applied_target_paths.push(target_vault_path.display().to_string());

        let app_json = obsidian_config::app_json_path(&target_vault_path);
        backup.backup_path(&app_json, "update-ignore-filters")?;
        if let Err(error) = obsidian_config::ensure_backup_dir_ignored(&target_vault_path) {
            results.push(error_result(
                None,
                &target_vault_path,
                "ignore-backup-dir",
                error,
            ));
        }

        let mut enabled_ids: BTreeSet<String> = target_inventory
            .enabled_plugin_ids
            .iter()
            .cloned()
            .collect();
        let mut enabled_changed = false;

        for operation in target_operations {
            let result = apply_plugin_operation(
                &target_vault_path,
                &source_inventory.plugins,
                &target_inventory.plugins,
                &operation,
                &mut backup,
                &mut enabled_ids,
                &mut enabled_changed,
            );
            match result {
                Ok(mut operation_results) => results.append(&mut operation_results),
                Err(error) => results.push(error_result(
                    Some(operation.plugin_id.clone()),
                    &target_vault_path,
                    "plugin-operation",
                    error,
                )),
            }
        }

        if enabled_changed {
            let community_path = obsidian_config::community_plugins_path(&target_vault_path);
            backup.backup_path(&community_path, "update-enabled-state")?;
            let ids: Vec<String> = enabled_ids.into_iter().collect();
            match obsidian_config::write_enabled_plugin_ids(&target_vault_path, &ids) {
                Ok(()) => results.push(OperationResult {
                    plugin_id: None,
                    target_vault_path: target_vault_path.display().to_string(),
                    action: "sync-enabled-state".to_string(),
                    status: OperationStatus::Success,
                    message: "已更新启用状态".to_string(),
                    path: Some(community_path.display().to_string()),
                }),
                Err(error) => results.push(error_result(
                    None,
                    &target_vault_path,
                    "sync-enabled-state",
                    error,
                )),
            }
        }

        let partial_summary = SyncSummary {
            started_at: started_at.clone(),
            finished_at: timestamp(),
            app_version: crate::models::current_app_version(),
            source_vault_path: Some(source_vault_path.display().to_string()),
            target_vault_paths: vec![target_vault_path.display().to_string()],
            backup_paths: vec![backup.path().display().to_string()],
            results: results
                .iter()
                .filter(|result| {
                    result.target_vault_path == target_vault_path.display().to_string()
                })
                .cloned()
                .collect(),
        };
        write_sync_reports(
            backup.path(),
            &partial_summary,
            "sync-report.json",
            "sync-report.md",
        )?;
    }

    Ok(SyncSummary {
        started_at,
        finished_at: timestamp(),
        app_version: crate::models::current_app_version(),
        source_vault_path: Some(source_vault_path.display().to_string()),
        target_vault_paths: applied_target_paths,
        backup_paths,
        results,
    })
}

fn validate_operation_shape(
    operation: &SelectedPluginOperation,
    source_vault_path: &str,
) -> AppResult<()> {
    if operation.plugin_id == BRIDGE_PLUGIN_ID {
        return Err(AppError::new(
            "settings_bridge_sync_forbidden",
            "Bridge 包含知识库本地运行时缓存，只能在各知识库中单独安装和管理",
        ));
    }
    if operation.source_vault_path != source_vault_path {
        return Err(AppError::new(
            "invalid_operation_source",
            "同步操作的源知识库不一致",
        ));
    }
    if operation.sync_enabled_state
        && !operation.copy_plugin_files
        && operation.delete_target_plugin
    {
        return Err(AppError::new(
            "invalid_operation",
            "删除插件时不能同步启用状态",
        ));
    }
    Ok(())
}

fn apply_plugin_operation(
    target_vault_path: &Path,
    source_plugins: &[PluginInventoryItem],
    target_plugins: &[PluginInventoryItem],
    operation: &SelectedPluginOperation,
    backup: &mut BackupSession,
    enabled_ids: &mut BTreeSet<String>,
    enabled_changed: &mut bool,
) -> AppResult<Vec<OperationResult>> {
    let mut results = Vec::new();
    let source_plugin = source_plugins
        .iter()
        .find(|plugin| plugin.id.as_deref() == Some(&operation.plugin_id));
    let target_plugin = target_plugins
        .iter()
        .find(|plugin| plugin.id.as_deref() == Some(&operation.plugin_id));

    if operation.delete_target_plugin {
        let target_plugin = target_plugin
            .ok_or_else(|| AppError::new("missing_target_plugin", "目标插件不存在，无法删除"))?;
        let target_dir = PathBuf::from(&target_plugin.folder_path);
        backup.backup_path(&target_dir, "delete-plugin")?;
        fs_safety::remove_path(&target_dir)?;
        enabled_ids.remove(&operation.plugin_id);
        *enabled_changed = true;
        results.push(success_result(
            &operation.plugin_id,
            target_vault_path,
            "delete-plugin",
            "已删除目标独有插件",
            Some(&target_dir),
        ));
        return Ok(results);
    }

    let source_plugin =
        source_plugin.ok_or_else(|| AppError::new("missing_source_plugin", "源插件不存在"))?;
    if !source_plugin.valid || source_plugin.unsupported_reason.is_some() {
        return Err(AppError::new(
            "unsupported_source_plugin",
            "源插件无效或不支持",
        ));
    }
    if operation.sync_enabled_state && target_plugin.is_none() && !operation.copy_plugin_files {
        return Err(AppError::new(
            "enabled_state_requires_plugin_copy",
            "目标插件不存在时，同步启用状态必须同时复制插件本体",
        ));
    }
    if operation.copy_plugin_files && !operation.force_downgrade {
        if let Some(target_plugin) = target_plugin {
            if compare_versions(
                source_plugin.version.as_deref(),
                target_plugin.version.as_deref(),
            ) == crate::models::PluginDiffStatus::SourceOlder
            {
                return Err(AppError::new(
                    "downgrade_requires_confirmation",
                    "降级操作需要用户显式确认",
                ));
            }
        }
    }

    if operation.copy_plugin_files {
        let target_dir = target_plugins_dir(target_vault_path).join(
            target_plugin
                .map(|plugin| plugin.folder_name.as_str())
                .unwrap_or(source_plugin.folder_name.as_str()),
        );
        backup.backup_path(&target_dir, "copy-plugin-files")?;
        let device_local_filtered = copy_plugin_directory(
            source_plugin,
            target_plugin,
            &target_dir,
            target_vault_path,
            operation.sync_data_json,
        )?;
        results.push(success_result(
            &operation.plugin_id,
            target_vault_path,
            "copy-plugin-files",
            if device_local_filtered {
                "已复制或更新插件本体，并保留目标设备本地设置"
            } else {
                "已复制或更新插件本体"
            },
            Some(&target_dir),
        ));
    }

    if operation.sync_data_json && !operation.copy_plugin_files {
        let source_data = PathBuf::from(&source_plugin.folder_path).join("data.json");
        let Some(target_plugin) = target_plugin else {
            results.push(skipped_result(
                &operation.plugin_id,
                target_vault_path,
                "sync-data-json",
                "目标插件不存在，已跳过设置同步",
                None,
            ));
            return Ok(results);
        };
        if !source_data.exists() {
            results.push(skipped_result(
                &operation.plugin_id,
                target_vault_path,
                "sync-data-json",
                "源插件没有 data.json，已跳过设置同步",
                Some(&source_data),
            ));
        } else {
            let target_dir = PathBuf::from(&target_plugin.folder_path);
            let target_data = target_dir.join("data.json");
            backup.backup_path(&target_dir, "sync-data-json")?;
            let device_local_filtered =
                copy_plugin_configuration(source_plugin, Some(target_plugin), &target_data, false)?;
            results.push(success_result(
                &operation.plugin_id,
                target_vault_path,
                "sync-data-json",
                if device_local_filtered {
                    "已同步插件设置，并保留目标设备本地字段"
                } else {
                    "已同步插件设置"
                },
                Some(&target_data),
            ));
        }
    }

    if operation.sync_enabled_state {
        if source_plugin.enabled {
            enabled_ids.insert(operation.plugin_id.clone());
        } else {
            enabled_ids.remove(&operation.plugin_id);
        }
        *enabled_changed = true;
        results.push(success_result(
            &operation.plugin_id,
            target_vault_path,
            "queue-enabled-state",
            "已加入启用状态更新队列",
            None,
        ));
    }

    Ok(results)
}

fn copy_plugin_directory(
    source_plugin: &PluginInventoryItem,
    target_plugin: Option<&PluginInventoryItem>,
    target_dir: &Path,
    target_vault_path: &Path,
    sync_data_json: bool,
) -> AppResult<bool> {
    let source_dir = PathBuf::from(&source_plugin.folder_path);
    let temp_name = format!(".ops-temp-{}-{}", timestamp(), source_plugin.folder_name);
    let stage_dir = target_plugins_dir(target_vault_path).join(temp_name);
    if stage_dir.exists() {
        fs_safety::remove_path(&stage_dir)?;
    }
    let result = (|| -> AppResult<bool> {
        fs_safety::copy_path_recursive(&source_dir, &stage_dir)?;

        let stage_data = stage_dir.join("data.json");
        let device_local_filtered = if sync_data_json && stage_data.exists() {
            copy_plugin_configuration(source_plugin, target_plugin, &stage_data, true)?
        } else {
            false
        };
        if !sync_data_json {
            if let Some(target_plugin) = target_plugin {
                let target_data = PathBuf::from(&target_plugin.folder_path).join("data.json");
                if target_data.exists() {
                    fs::copy(&target_data, &stage_data)
                        .map_err(|error| AppError::from(error).with_path(&stage_data))?;
                } else if stage_data.exists() {
                    fs::remove_file(&stage_data)
                        .map_err(|error| AppError::from(error).with_path(&stage_data))?;
                }
            } else if stage_data.exists() {
                fs::remove_file(&stage_data)
                    .map_err(|error| AppError::from(error).with_path(&stage_data))?;
            }
        }

        fs_safety::replace_dir_with_stage(&stage_dir, target_dir)?;
        Ok(device_local_filtered)
    })();
    if result.is_err() && stage_dir.exists() {
        let _ = fs_safety::remove_path(&stage_dir);
    }
    result
}

fn copy_plugin_configuration(
    source_plugin: &PluginInventoryItem,
    target_plugin: Option<&PluginInventoryItem>,
    destination: &Path,
    copy_plugin_files: bool,
) -> AppResult<bool> {
    let source_data = PathBuf::from(&source_plugin.folder_path).join("data.json");
    if fs_safety::is_link_path(&source_data)? {
        return Err(
            AppError::new("unsupported_link_path", "不支持同步链接形式的 data.json")
                .with_path(source_data),
        );
    }
    if !should_filter_configuration_for_sync(source_plugin, target_plugin, copy_plugin_files) {
        fs::copy(&source_data, destination)
            .map_err(|error| AppError::from(error).with_path(destination))?;
        return Ok(false);
    }

    let source_configuration = read_sync_configuration(&source_data, "源")?;
    let target_configuration = target_plugin
        .map(|plugin| PathBuf::from(&plugin.folder_path).join("data.json"))
        .filter(|path| path.exists())
        .map(|path| read_sync_configuration(&path, "目标"))
        .transpose()?;
    let prepared = prepare_configuration_for_sync(
        source_plugin,
        target_plugin,
        copy_plugin_files,
        &source_configuration,
        target_configuration.as_ref(),
    )?
    .ok_or_else(|| {
        AppError::new(
            "plugin_adapter_sync_state_changed",
            "适配器同步条件在写入前发生变化，已停止本次设置同步",
        )
    })?;
    obsidian_config::write_json_atomic(destination, &prepared)?;
    Ok(true)
}

fn read_sync_configuration(path: &Path, side: &str) -> AppResult<Value> {
    if fs_safety::is_link_path(path)? {
        return Err(AppError::new(
            "unsupported_link_path",
            format!("{side} data.json 不能是链接"),
        )
        .with_path(path));
    }
    let content =
        fs::read_to_string(path).map_err(|error| AppError::from(error).with_path(path))?;
    serde_json::from_str(&content).map_err(|error| {
        AppError::new(
            "plugin_adapter_sync_configuration_invalid",
            format!("{side} data.json 不是有效 JSON，无法保护设备本地设置"),
        )
        .with_path(path)
        .with_details(error.to_string())
    })
}

fn target_plugins_dir(target_vault_path: &Path) -> PathBuf {
    obsidian_config::plugins_dir(target_vault_path)
}

fn success_result(
    plugin_id: &str,
    target_vault_path: &Path,
    action: &str,
    message: &str,
    path: Option<&Path>,
) -> OperationResult {
    OperationResult {
        plugin_id: Some(plugin_id.to_string()),
        target_vault_path: target_vault_path.display().to_string(),
        action: action.to_string(),
        status: OperationStatus::Success,
        message: message.to_string(),
        path: path.map(|path| path.display().to_string()),
    }
}

fn skipped_result(
    plugin_id: &str,
    target_vault_path: &Path,
    action: &str,
    message: &str,
    path: Option<&Path>,
) -> OperationResult {
    OperationResult {
        plugin_id: Some(plugin_id.to_string()),
        target_vault_path: target_vault_path.display().to_string(),
        action: action.to_string(),
        status: OperationStatus::Skipped,
        message: message.to_string(),
        path: path.map(|path| path.display().to_string()),
    }
}

fn error_result(
    plugin_id: Option<String>,
    target_vault_path: &Path,
    action: &str,
    error: AppError,
) -> OperationResult {
    OperationResult {
        plugin_id,
        target_vault_path: target_vault_path.display().to_string(),
        action: action.to_string(),
        status: OperationStatus::Failed,
        message: error.message,
        path: error.path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{
        env,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn temp_root(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = env::temp_dir().join(format!("obsidian-plugin-sync-{name}-{unique}"));
        fs::create_dir_all(&root).expect("fixture root");
        root
    }

    fn plugin(root: &Path, folder: &str, version: &str) -> PluginInventoryItem {
        let plugin_dir = root.join(folder);
        fs::create_dir_all(&plugin_dir).expect("plugin dir");
        PluginInventoryItem {
            id: Some("realclaudian".to_string()),
            folder_name: folder.to_string(),
            folder_path: plugin_dir.display().to_string(),
            manifest_path: plugin_dir.join("manifest.json").display().to_string(),
            name: Some("Claudian".to_string()),
            version: Some(version.to_string()),
            enabled: true,
            has_data_json: true,
            valid: true,
            unsupported_reason: None,
            warnings: Vec::new(),
        }
    }

    #[test]
    fn rejects_bridge_from_normal_sync_operations() {
        let operation = SelectedPluginOperation {
            plugin_id: BRIDGE_PLUGIN_ID.to_string(),
            source_vault_path: "C:/source".to_string(),
            target_vault_path: "C:/target".to_string(),
            copy_plugin_files: true,
            sync_data_json: true,
            sync_enabled_state: true,
            delete_target_plugin: false,
            force_downgrade: false,
        };

        let error = validate_operation_shape(&operation, "C:/source")
            .expect_err("Bridge must remain local to each vault");
        assert_eq!(error.code, "settings_bridge_sync_forbidden");
    }

    #[test]
    fn adapted_sync_keeps_target_device_local_configuration() {
        let root = temp_root("keep-target-device");
        let source = plugin(&root, "source", "2.0.24");
        let target = plugin(&root, "target", "2.0.24");
        fs::write(
            PathBuf::from(&source.folder_path).join("data.json"),
            serde_json::to_string(&json!({
                "locale": "zh-cn",
                "providerConfigs": {
                    "claude": {
                        "safeMode": "auto",
                        "cliPathsByHost": {"device:source": "C:/source.exe"}
                    }
                }
            }))
            .unwrap(),
        )
        .unwrap();
        let target_data = PathBuf::from(&target.folder_path).join("data.json");
        fs::write(
            &target_data,
            serde_json::to_string(&json!({
                "locale": "en",
                "providerConfigs": {
                    "claude": {
                        "safeMode": "default",
                        "cliPathsByHost": {"device:target": "D:/target.exe"}
                    }
                }
            }))
            .unwrap(),
        )
        .unwrap();

        assert!(copy_plugin_configuration(&source, Some(&target), &target_data, false).unwrap());
        let saved = read_sync_configuration(&target_data, "test").unwrap();
        assert_eq!(saved.pointer("/locale"), Some(&json!("zh-cn")));
        assert_eq!(
            saved.pointer("/providerConfigs/claude/safeMode"),
            Some(&json!("auto"))
        );
        assert_eq!(
            saved.pointer("/providerConfigs/claude/cliPathsByHost"),
            Some(&json!({"device:target": "D:/target.exe"}))
        );

        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn adapted_copy_to_new_plugin_removes_source_device_values() {
        let root = temp_root("new-target-device");
        let source = plugin(&root, "source", "2.0.24");
        fs::write(
            PathBuf::from(&source.folder_path).join("data.json"),
            serde_json::to_string(&json!({
                "locale": "zh-cn",
                "providerConfigs": {
                    "pi": {
                        "enabled": true,
                        "cliPathsByHost": {"device:source": "C:/pi.exe"}
                    }
                }
            }))
            .unwrap(),
        )
        .unwrap();
        let stage = root.join("stage");
        fs::create_dir_all(&stage).unwrap();
        let destination = stage.join("data.json");

        assert!(copy_plugin_configuration(&source, None, &destination, true).unwrap());
        let saved = read_sync_configuration(&destination, "test").unwrap();
        assert!(saved
            .pointer("/providerConfigs/pi/cliPathsByHost")
            .is_none());
        assert_eq!(
            saved.pointer("/providerConfigs/pi/enabled"),
            Some(&json!(true))
        );

        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn malformed_target_configuration_blocks_adapted_sync() {
        let root = temp_root("invalid-target-device");
        let source = plugin(&root, "source", "2.0.24");
        let target = plugin(&root, "target", "2.0.24");
        fs::write(
            PathBuf::from(&source.folder_path).join("data.json"),
            r#"{"locale":"zh-cn"}"#,
        )
        .unwrap();
        let target_data = PathBuf::from(&target.folder_path).join("data.json");
        fs::write(&target_data, "{broken").unwrap();

        let error = copy_plugin_configuration(&source, Some(&target), &target_data, false)
            .expect_err("invalid target must block adapted sync");
        assert_eq!(error.code, "plugin_adapter_sync_configuration_invalid");
        assert_eq!(fs::read_to_string(&target_data).unwrap(), "{broken");

        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn failed_adapted_plugin_copy_removes_stage_directory() {
        let root = temp_root("cleanup-failed-stage");
        let source_root = root.join("source-vault");
        let target_vault = root.join("target-vault");
        fs::create_dir_all(target_vault.join(".obsidian/plugins")).unwrap();
        let source = plugin(&source_root, "realclaudian", "2.0.24");
        let target_dir = target_vault.join(".obsidian/plugins/realclaudian");
        fs::create_dir_all(&target_dir).unwrap();
        let mut target = plugin(
            &target_vault.join(".obsidian/plugins"),
            "realclaudian",
            "2.0.24",
        );
        target.folder_path = target_dir.display().to_string();
        fs::write(PathBuf::from(&source.folder_path).join("main.js"), "source").unwrap();
        fs::write(
            PathBuf::from(&source.folder_path).join("data.json"),
            r#"{"locale":"zh-cn"}"#,
        )
        .unwrap();
        fs::write(target_dir.join("data.json"), "{broken").unwrap();

        copy_plugin_directory(&source, Some(&target), &target_dir, &target_vault, true)
            .expect_err("invalid target must abort copy");
        let plugin_entries = fs::read_dir(target_vault.join(".obsidian/plugins"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
            .collect::<Vec<_>>();
        assert!(plugin_entries
            .iter()
            .all(|name| !name.starts_with(".ops-temp-")));

        fs::remove_dir_all(root).expect("cleanup");
    }
}
