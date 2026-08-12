use crate::{
    errors::{AppError, AppResult},
    fs_safety,
    models::{BackupInfo, OperationResult, OperationStatus, SyncSummary},
    obsidian_config,
    process::obsidian_is_running,
    reports::write_sync_reports,
};
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupManifest {
    pub created_at: String,
    pub kind: String,
    pub vault_path: String,
    pub entries: Vec<BackupEntry>,
    #[serde(default)]
    pub plugin_context: Option<PluginBackupContext>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginBackupContext {
    pub plugin_id: String,
    pub operation: String,
    pub enabled_before: bool,
    pub plugin_directory: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupEntry {
    pub original_path: String,
    pub backup_relative_path: Option<String>,
    pub original_existed: bool,
    pub path_kind: String,
    pub reason: String,
}

pub struct BackupSession {
    pub dir: PathBuf,
    manifest: BackupManifest,
}

impl BackupSession {
    pub fn create(vault_path: &Path, kind: &str) -> AppResult<Self> {
        let timestamp = timestamp();
        let root = obsidian_config::backup_root(vault_path);
        let mut dir = root.join(&timestamp);
        let mut suffix = 1;
        while dir.exists() {
            dir = root.join(format!("{timestamp}-{suffix:02}"));
            suffix += 1;
        }
        fs::create_dir_all(&dir).map_err(|error| AppError::from(error).with_path(&dir))?;
        let manifest = BackupManifest {
            created_at: timestamp,
            kind: kind.to_string(),
            vault_path: vault_path.display().to_string(),
            entries: Vec::new(),
            plugin_context: None,
        };
        let session = Self { dir, manifest };
        session.save_manifest()?;
        Ok(session)
    }

    pub fn set_plugin_context(&mut self, context: PluginBackupContext) -> AppResult<()> {
        self.manifest.plugin_context = Some(context);
        self.save_manifest()
    }

    pub fn backup_path(&mut self, path: &Path, reason: &str) -> AppResult<()> {
        let vault_path = PathBuf::from(&self.manifest.vault_path);
        fs_safety::ensure_child_path(&vault_path, path)?;
        let existed = path.exists();
        let path_kind = if existed && path.is_dir() {
            "directory"
        } else {
            "file"
        }
        .to_string();
        let backup_relative_path = if existed {
            let relative = path.strip_prefix(&vault_path).map_err(|_| {
                AppError::new("path_outside_vault", "备份路径不在知识库内").with_path(path)
            })?;
            let backup_relative = PathBuf::from("files").join(relative);
            let backup_absolute = self.dir.join(&backup_relative);
            fs_safety::copy_path_recursive(path, &backup_absolute)?;
            Some(backup_relative.display().to_string())
        } else {
            None
        };

        self.manifest.entries.push(BackupEntry {
            original_path: path.display().to_string(),
            backup_relative_path,
            original_existed: existed,
            path_kind,
            reason: reason.to_string(),
        });
        self.save_manifest()
    }

    pub fn save_manifest(&self) -> AppResult<()> {
        let path = self.dir.join("backup-manifest.json");
        let content = serde_json::to_string_pretty(&self.manifest)
            .map_err(|error| AppError::from(error).with_path(&path))?;
        fs::write(&path, content).map_err(|error| AppError::from(error).with_path(&path))
    }

    pub fn path(&self) -> &Path {
        &self.dir
    }
}

pub fn list_backup_infos(vault_path: String) -> AppResult<Vec<BackupInfo>> {
    let vault_path = PathBuf::from(vault_path);
    let backup_root = obsidian_config::backup_root(&vault_path);
    if !backup_root.exists() {
        return Ok(Vec::new());
    }
    let mut backups = Vec::new();
    for entry in
        fs::read_dir(&backup_root).map_err(|error| AppError::from(error).with_path(&backup_root))?
    {
        let entry = entry.map_err(|error| AppError::from(error).with_path(&backup_root))?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let created_at = entry.file_name().to_string_lossy().to_string();
        let report_path = path.join("sync-report.json");
        let manifest = read_manifest(&path).ok();
        let plugin_context = manifest
            .as_ref()
            .and_then(|manifest| manifest.plugin_context.as_ref());
        backups.push(BackupInfo {
            vault_path: vault_path.display().to_string(),
            backup_path: path.display().to_string(),
            created_at,
            report_path: report_path
                .exists()
                .then(|| report_path.display().to_string()),
            kind: manifest.as_ref().map(|manifest| manifest.kind.clone()),
            plugin_id: plugin_context.map(|context| context.plugin_id.clone()),
            operation: plugin_context.map(|context| context.operation.clone()),
        });
    }
    backups.sort_by(|left, right| right.created_at.cmp(&left.created_at));
    Ok(backups)
}

pub fn restore_backup_dir(
    vault_path: String,
    backup_path: String,
    obsidian_closed_confirmed: bool,
) -> AppResult<SyncSummary> {
    if !obsidian_closed_confirmed {
        return Err(AppError::new(
            "obsidian_not_confirmed_closed",
            "请先确认 Obsidian 已关闭",
        ));
    }
    if obsidian_is_running()? {
        return Err(AppError::new(
            "obsidian_running",
            "检测到 Obsidian.exe 正在运行，请关闭后再恢复",
        ));
    }

    restore_backup_dir_after_gate(vault_path, backup_path)
}

pub(crate) fn restore_backup_dir_after_gate(
    vault_path: String,
    backup_path: String,
) -> AppResult<SyncSummary> {
    let started_at = timestamp();
    let vault_path = PathBuf::from(vault_path);
    let backup_path = PathBuf::from(backup_path);
    let manifest = read_manifest(&backup_path)?;
    if manifest.plugin_context.is_some() {
        return restore_plugin_backup_manifest(vault_path, backup_path, manifest);
    }
    let mut pre_restore = BackupSession::create(&vault_path, "pre-restore")?;
    let mut results = Vec::new();

    for entry in &manifest.entries {
        let original = PathBuf::from(&entry.original_path);
        pre_restore.backup_path(&original, "restore-preimage")?;
        let result = restore_entry(&backup_path, entry);
        results.push(match result {
            Ok(message) => OperationResult {
                plugin_id: None,
                target_vault_path: vault_path.display().to_string(),
                action: "restore".to_string(),
                status: OperationStatus::Success,
                message,
                path: Some(original.display().to_string()),
            },
            Err(error) => OperationResult {
                plugin_id: None,
                target_vault_path: vault_path.display().to_string(),
                action: "restore".to_string(),
                status: OperationStatus::Failed,
                message: error.message,
                path: Some(original.display().to_string()),
            },
        });
    }

    let finished_at = timestamp();
    let summary = SyncSummary {
        started_at,
        finished_at,
        app_version: crate::models::current_app_version(),
        source_vault_path: None,
        target_vault_paths: vec![vault_path.display().to_string()],
        backup_paths: vec![pre_restore.path().display().to_string()],
        results,
    };
    write_sync_reports(
        pre_restore.path(),
        &summary,
        "restore-report.json",
        "restore-report.md",
    )?;
    Ok(summary)
}

fn restore_plugin_backup_manifest(
    vault_path: PathBuf,
    backup_path: PathBuf,
    manifest: BackupManifest,
) -> AppResult<SyncSummary> {
    let vault_path = fs_safety::canonical_existing(&vault_path)?;
    let manifest_vault_path = fs_safety::canonical_existing(Path::new(&manifest.vault_path))?;
    fs_safety::ensure_child_path(&vault_path, &backup_path)?;
    let context = manifest.plugin_context.clone().ok_or_else(|| {
        AppError::new(
            "missing_plugin_backup_context",
            "插件备份缺少单插件恢复信息",
        )
    })?;
    if manifest_vault_path != vault_path {
        return Err(AppError::new(
            "backup_vault_mismatch",
            "备份不属于当前知识库",
        ));
    }

    let plugin_dir = PathBuf::from(&context.plugin_directory);
    fs_safety::ensure_child_path(&vault_path, &plugin_dir)?;
    if fs_safety::is_link_path(&plugin_dir)? {
        return Err(
            AppError::new("unsupported_link_path", "不支持恢复链接目录插件").with_path(&plugin_dir),
        );
    }
    let plugin_entry = manifest
        .entries
        .iter()
        .find(|entry| entry.original_path == context.plugin_directory)
        .ok_or_else(|| AppError::new("missing_plugin_backup_entry", "插件备份缺少插件目录快照"))?;

    let started_at = timestamp();
    let enabled_ids = obsidian_config::read_enabled_plugin_ids(&vault_path)?;
    let enabled_before_restore = enabled_ids.iter().any(|id| id == &context.plugin_id);
    let mut pre_restore = BackupSession::create(&vault_path, "plugin-management-pre-restore")?;
    pre_restore.set_plugin_context(PluginBackupContext {
        plugin_id: context.plugin_id.clone(),
        operation: "restore".to_string(),
        enabled_before: enabled_before_restore,
        plugin_directory: context.plugin_directory.clone(),
    })?;
    pre_restore.backup_path(&plugin_dir, "plugin-directory")?;
    pre_restore.backup_path(
        &obsidian_config::community_plugins_path(&vault_path),
        "enabled-state-safety-snapshot",
    )?;
    pre_restore.backup_path(
        &obsidian_config::app_json_path(&vault_path),
        "app-json-safety-snapshot",
    )?;

    let restore_result = (|| -> AppResult<String> {
        restore_entry(&backup_path, plugin_entry)?;
        let mut enabled: std::collections::BTreeSet<String> =
            obsidian_config::read_enabled_plugin_ids(&vault_path)?
                .into_iter()
                .collect();
        if context.enabled_before {
            enabled.insert(context.plugin_id.clone());
        } else {
            enabled.remove(&context.plugin_id);
        }
        obsidian_config::write_enabled_plugin_ids(
            &vault_path,
            &enabled.into_iter().collect::<Vec<_>>(),
        )?;
        Ok("已恢复该插件及其启用状态".to_string())
    })();

    let result = match restore_result {
        Ok(message) => OperationResult {
            plugin_id: Some(context.plugin_id.clone()),
            target_vault_path: vault_path.display().to_string(),
            action: "restore-plugin".to_string(),
            status: OperationStatus::Success,
            message,
            path: Some(plugin_dir.display().to_string()),
        },
        Err(error) => OperationResult {
            plugin_id: Some(context.plugin_id.clone()),
            target_vault_path: vault_path.display().to_string(),
            action: "restore-plugin".to_string(),
            status: OperationStatus::Failed,
            message: error.message,
            path: error
                .path
                .or_else(|| Some(plugin_dir.display().to_string())),
        },
    };
    let summary = SyncSummary {
        started_at,
        finished_at: timestamp(),
        app_version: crate::models::current_app_version(),
        source_vault_path: None,
        target_vault_paths: vec![vault_path.display().to_string()],
        backup_paths: vec![pre_restore.path().display().to_string()],
        results: vec![result],
    };
    write_sync_reports(
        pre_restore.path(),
        &summary,
        "sync-report.json",
        "sync-report.md",
    )?;
    Ok(summary)
}

fn restore_entry(backup_dir: &Path, entry: &BackupEntry) -> AppResult<String> {
    let original = PathBuf::from(&entry.original_path);
    if entry.original_existed {
        let relative = entry
            .backup_relative_path
            .as_ref()
            .ok_or_else(|| AppError::new("invalid_backup_manifest", "备份清单缺少备份路径"))?;
        let backed_up = backup_dir.join(relative);
        if original.exists() {
            fs_safety::remove_path(&original)?;
        }
        fs_safety::copy_path_recursive(&backed_up, &original)?;
        Ok("已恢复备份内容".to_string())
    } else {
        if original.exists() {
            fs_safety::remove_path(&original)?;
        }
        Ok("已移除同步新增路径".to_string())
    }
}

pub fn read_manifest(backup_dir: &Path) -> AppResult<BackupManifest> {
    let path = backup_dir.join("backup-manifest.json");
    let content =
        fs::read_to_string(&path).map_err(|error| AppError::from(error).with_path(&path))?;
    serde_json::from_str(&content).map_err(|error| AppError::from(error).with_path(&path))
}

pub fn timestamp() -> String {
    Local::now().format("%Y-%m-%d_%H-%M-%S").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{models::current_app_version, reports::write_sync_reports};
    use std::{
        env,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn temp_root(name: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir().join(format!("ops-backup-{name}-{}-{suffix}", std::process::id()))
    }

    fn write_plugin(dir: &Path, body: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join("main.js"), body).unwrap();
        fs::write(
            dir.join("manifest.json"),
            r#"{"id":"demo","name":"Demo","version":"1.0.0"}"#,
        )
        .unwrap();
    }

    #[test]
    fn restore_entry_restores_modified_plugin_directory() {
        let root = temp_root("restore-modified");
        let vault = root.join("vault");
        let plugins = vault.join(".obsidian").join("plugins");
        let plugin = plugins.join("demo");
        write_plugin(&plugin, "console.log('original')");

        let mut session = BackupSession::create(&vault, "sync").unwrap();
        session.backup_path(&plugin, "modify-plugin").unwrap();
        fs::write(plugin.join("main.js"), "console.log('mutated')").unwrap();

        let manifest = read_manifest(session.path()).unwrap();
        let entry = manifest
            .entries
            .iter()
            .find(|item| item.original_path == plugin.display().to_string())
            .expect("plugin entry");
        let message = restore_entry(session.path(), entry).unwrap();
        assert!(message.contains("恢复"));
        assert_eq!(
            fs::read_to_string(plugin.join("main.js")).unwrap(),
            "console.log('original')"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn restore_entry_removes_paths_added_by_sync() {
        let root = temp_root("restore-added");
        let vault = root.join("vault");
        fs::create_dir_all(vault.join(".obsidian")).unwrap();
        let added = vault
            .join(".obsidian")
            .join("plugins")
            .join("added-by-sync");

        let mut session = BackupSession::create(&vault, "sync").unwrap();
        // Path did not exist at backup time (sync later added it).
        session.backup_path(&added, "add-plugin").unwrap();
        write_plugin(&added, "console.log('new')");
        assert!(added.exists());

        let manifest = read_manifest(session.path()).unwrap();
        let entry = manifest
            .entries
            .iter()
            .find(|item| item.original_path == added.display().to_string())
            .expect("added entry");
        assert!(!entry.original_existed);
        assert!(entry.backup_relative_path.is_none());

        let message = restore_entry(session.path(), entry).unwrap();
        assert!(message.contains("移除"));
        assert!(!added.exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn restore_entry_restores_deleted_plugin_directory() {
        let root = temp_root("restore-deleted");
        let vault = root.join("vault");
        let plugin = vault.join(".obsidian").join("plugins").join("to-delete");
        write_plugin(&plugin, "console.log('keep-me')");

        let mut session = BackupSession::create(&vault, "sync").unwrap();
        session.backup_path(&plugin, "delete-plugin").unwrap();
        fs_safety::remove_path(&plugin).unwrap();
        assert!(!plugin.exists());

        let manifest = read_manifest(session.path()).unwrap();
        let entry = manifest
            .entries
            .iter()
            .find(|item| item.original_path == plugin.display().to_string())
            .expect("deleted entry");
        restore_entry(session.path(), entry).unwrap();
        assert!(plugin.exists());
        assert_eq!(
            fs::read_to_string(plugin.join("main.js")).unwrap(),
            "console.log('keep-me')"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn sync_report_includes_app_version() {
        let root = temp_root("report-version");
        fs::create_dir_all(&root).unwrap();
        let summary = SyncSummary {
            started_at: "t0".into(),
            finished_at: "t1".into(),
            app_version: current_app_version(),
            source_vault_path: Some("C:/source".into()),
            target_vault_paths: vec!["C:/target".into()],
            backup_paths: vec![root.display().to_string()],
            results: Vec::new(),
        };
        write_sync_reports(&root, &summary, "sync-report.json", "sync-report.md").unwrap();

        let json = fs::read_to_string(root.join("sync-report.json")).unwrap();
        let md = fs::read_to_string(root.join("sync-report.md")).unwrap();
        assert!(json.contains(&current_app_version()));
        assert!(md.contains(&format!("应用版本：{}", current_app_version())));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn plugin_restore_preserves_other_plugins_enabled_state() {
        let root = temp_root("plugin-restore-isolated");
        let vault = root.join("vault");
        let plugin = vault.join(".obsidian/plugins/demo");
        write_plugin(&plugin, "console.log('original')");
        fs::write(
            vault.join(".obsidian/community-plugins.json"),
            r#"["demo","other"]"#,
        )
        .unwrap();
        fs::write(vault.join(".obsidian/app.json"), r#"{"theme":"dark"}"#).unwrap();

        let canonical_vault = fs::canonicalize(&vault).unwrap();
        let canonical_plugin = canonical_vault.join(".obsidian/plugins/demo");
        let mut session = BackupSession::create(&canonical_vault, "plugin-management").unwrap();
        session
            .set_plugin_context(PluginBackupContext {
                plugin_id: "demo".into(),
                operation: "save-configuration".into(),
                enabled_before: true,
                plugin_directory: canonical_plugin.display().to_string(),
            })
            .unwrap();
        session
            .backup_path(&canonical_plugin, "plugin-directory")
            .unwrap();
        session
            .backup_path(
                &canonical_vault.join(".obsidian/community-plugins.json"),
                "enabled-state-safety-snapshot",
            )
            .unwrap();
        session
            .backup_path(
                &canonical_vault.join(".obsidian/app.json"),
                "app-json-safety-snapshot",
            )
            .unwrap();

        fs::write(plugin.join("main.js"), "console.log('changed')").unwrap();
        fs::write(
            vault.join(".obsidian/community-plugins.json"),
            r#"["other","later"]"#,
        )
        .unwrap();

        let backup_path = session.path().to_path_buf();
        let manifest = read_manifest(&backup_path).unwrap();
        let summary = restore_plugin_backup_manifest(vault.clone(), backup_path, manifest).unwrap();
        assert!(matches!(
            summary.results.first().map(|result| &result.status),
            Some(OperationStatus::Success)
        ));
        assert_eq!(
            fs::read_to_string(plugin.join("main.js")).unwrap(),
            "console.log('original')"
        );
        let enabled = obsidian_config::read_enabled_plugin_ids(&vault).unwrap();
        assert!(enabled.contains(&"demo".to_string()));
        assert!(enabled.contains(&"other".to_string()));
        assert!(enabled.contains(&"later".to_string()));

        let _ = fs::remove_dir_all(root);
    }
}
