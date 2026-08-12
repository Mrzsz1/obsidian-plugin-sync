use crate::{
    backup::timestamp,
    errors::{AppError, AppResult},
    fs_safety,
    models::{
        PluginInventoryItem, RawConfigDiffEntry, RawConfigDiffOperation, RawConfigDiffPreview,
        RawPluginConfiguration, SyncSummary,
    },
    obsidian_config,
    plugin_manager::{
        begin_plugin_backup, ensure_write_allowed, find_supported_plugin, finish_operation,
        validate_plugin_id,
    },
    vault::scan_vault_inventory,
};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

pub fn inspect_raw_plugin_configuration(
    vault_path: String,
    plugin_id: String,
) -> AppResult<RawPluginConfiguration> {
    let (_, _, data_path) = resolve_raw_plugin(vault_path, &plugin_id)?;
    read_raw_configuration(&plugin_id, &data_path)
}

pub fn preview_raw_plugin_configuration(
    vault_path: String,
    plugin_id: String,
    proposed: Value,
) -> AppResult<RawConfigDiffPreview> {
    let current = inspect_raw_plugin_configuration(vault_path, plugin_id)?;
    Ok(build_raw_diff(&current, &proposed))
}

pub fn save_raw_plugin_configuration(
    vault_path: String,
    plugin_id: String,
    proposed: Value,
    expected_current_revision: String,
    raw_risk_confirmed: bool,
    obsidian_closed_confirmed: bool,
) -> AppResult<SyncSummary> {
    if !raw_risk_confirmed {
        return Err(AppError::new(
            "raw_configuration_confirmation_required",
            "原始配置编辑需要单独确认高风险操作",
        ));
    }
    ensure_write_allowed(obsidian_closed_confirmed)?;
    save_raw_plugin_configuration_after_gate(
        vault_path,
        plugin_id,
        proposed,
        expected_current_revision,
    )
}

fn save_raw_plugin_configuration_after_gate(
    vault_path: String,
    plugin_id: String,
    proposed: Value,
    expected_current_revision: String,
) -> AppResult<SyncSummary> {
    let (vault_root, plugin, data_path) = resolve_raw_plugin(vault_path, &plugin_id)?;
    let current = read_raw_configuration(&plugin_id, &data_path)?;
    if current.revision != expected_current_revision {
        return Err(AppError::new(
            "raw_configuration_changed_since_preview",
            "data.json 在差异预览后发生变化，请刷新并重新检查差异",
        )
        .with_path(data_path));
    }
    let preview = build_raw_diff(&current, &proposed);
    if preview.entries.is_empty() {
        return Err(AppError::new(
            "raw_configuration_unchanged",
            "原始配置没有变化",
        ));
    }

    let plugin_dir = PathBuf::from(&plugin.folder_path);
    let started_at = timestamp();
    let backup = begin_plugin_backup(
        &vault_root,
        &plugin_id,
        "save-raw-configuration",
        &plugin_dir,
        plugin.enabled,
    )?;
    let changed_count = preview.entries.len();
    let outcome = obsidian_config::write_json_atomic(&data_path, &proposed).map(|_| {
        (
            format!("已保存原始配置，共变更 {changed_count} 个 JSON 路径"),
            Some(data_path),
        )
    });
    finish_operation(
        started_at,
        &vault_root,
        &plugin_id,
        "save-plugin-raw-configuration",
        backup,
        outcome,
    )
}

fn resolve_raw_plugin(
    vault_path: String,
    plugin_id: &str,
) -> AppResult<(PathBuf, PluginInventoryItem, PathBuf)> {
    validate_plugin_id(plugin_id)?;
    let inventory = scan_vault_inventory(vault_path)?;
    let vault_root = PathBuf::from(&inventory.vault.path);
    let plugin = find_supported_plugin(&inventory.plugins, plugin_id)?.clone();
    let plugin_dir = PathBuf::from(&plugin.folder_path);
    fs_safety::ensure_child_path(&vault_root, &plugin_dir)?;
    if fs_safety::is_link_path(&plugin_dir)? {
        return Err(
            AppError::new("unsupported_link_path", "不支持读取链接目录插件的原始配置")
                .with_path(plugin_dir),
        );
    }
    let data_path = plugin_dir.join("data.json");
    fs_safety::ensure_child_path(&vault_root, &data_path)?;
    if fs_safety::is_link_path(&data_path)? {
        return Err(
            AppError::new("unsupported_link_path", "不支持读取链接形式的 data.json")
                .with_path(data_path),
        );
    }
    Ok((vault_root, plugin, data_path))
}

fn read_raw_configuration(plugin_id: &str, path: &Path) -> AppResult<RawPluginConfiguration> {
    if !path.exists() {
        return Ok(RawPluginConfiguration {
            plugin_id: plugin_id.to_string(),
            exists: false,
            byte_length: 0,
            revision: raw_revision(false, &[]),
            raw_text: "{}".to_string(),
            value: Some(Value::Object(Default::default())),
            parse_error: None,
        });
    }
    let bytes = fs::read(path).map_err(|error| AppError::from(error).with_path(path))?;
    let byte_length = bytes.len();
    let revision = raw_revision(true, &bytes);
    match String::from_utf8(bytes) {
        Ok(raw_text) => match serde_json::from_str::<Value>(&raw_text) {
            Ok(value) => Ok(RawPluginConfiguration {
                plugin_id: plugin_id.to_string(),
                exists: true,
                byte_length,
                revision,
                raw_text,
                value: Some(value),
                parse_error: None,
            }),
            Err(error) => Ok(RawPluginConfiguration {
                plugin_id: plugin_id.to_string(),
                exists: true,
                byte_length,
                revision,
                raw_text,
                value: None,
                parse_error: Some(format!("data.json 不是有效 JSON：{error}")),
            }),
        },
        Err(error) => Ok(RawPluginConfiguration {
            plugin_id: plugin_id.to_string(),
            exists: true,
            byte_length,
            revision,
            raw_text: String::from_utf8_lossy(error.as_bytes()).into_owned(),
            value: None,
            parse_error: Some(
                "data.json 不是有效 UTF-8，必须提供完整有效 JSON 才能替换".to_string(),
            ),
        }),
    }
}

fn build_raw_diff(current: &RawPluginConfiguration, proposed: &Value) -> RawConfigDiffPreview {
    let mut entries = Vec::new();
    if !current.exists {
        collect_diff(None, Some(proposed), "", false, &mut entries);
    } else if let Some(value) = current.value.as_ref() {
        collect_diff(Some(value), Some(proposed), "", false, &mut entries);
    } else {
        entries.push(RawConfigDiffEntry {
            path: "".to_string(),
            operation: RawConfigDiffOperation::Change,
            before_exists: true,
            before: Value::Null,
            after_exists: true,
            after: proposed.clone(),
            sensitive: false,
        });
    }
    RawConfigDiffPreview {
        plugin_id: current.plugin_id.clone(),
        current_exists: current.exists,
        current_revision: current.revision.clone(),
        current_parse_error: current.parse_error.clone(),
        entries,
    }
}

fn raw_revision(exists: bool, bytes: &[u8]) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{}-{}-{hash:016x}", u8::from(exists), bytes.len())
}

fn collect_diff(
    current: Option<&Value>,
    proposed: Option<&Value>,
    path: &str,
    sensitive: bool,
    output: &mut Vec<RawConfigDiffEntry>,
) {
    if current == proposed {
        return;
    }
    match (current, proposed) {
        (Some(Value::Object(current)), Some(Value::Object(proposed))) => {
            let keys = current
                .keys()
                .chain(proposed.keys())
                .cloned()
                .collect::<BTreeSet<_>>();
            for key in keys {
                collect_diff(
                    current.get(&key),
                    proposed.get(&key),
                    &append_pointer(path, &key),
                    sensitive || is_sensitive_key(&key),
                    output,
                );
            }
        }
        (Some(Value::Array(current)), Some(Value::Array(proposed))) => {
            for index in 0..current.len().max(proposed.len()) {
                collect_diff(
                    current.get(index),
                    proposed.get(index),
                    &append_pointer(path, &index.to_string()),
                    sensitive,
                    output,
                );
            }
        }
        (None, Some(value)) => collect_added(value, path, sensitive, output),
        (Some(value), None) => collect_removed(value, path, sensitive, output),
        (Some(before), Some(after)) => push_diff(
            path,
            RawConfigDiffOperation::Change,
            Some(before),
            Some(after),
            sensitive,
            output,
        ),
        (None, None) => {}
    }
}

fn collect_added(value: &Value, path: &str, sensitive: bool, output: &mut Vec<RawConfigDiffEntry>) {
    match value {
        Value::Object(object) if !object.is_empty() => {
            for (key, child) in object {
                collect_added(
                    child,
                    &append_pointer(path, key),
                    sensitive || is_sensitive_key(key),
                    output,
                );
            }
        }
        Value::Array(items) if !items.is_empty() => {
            for (index, child) in items.iter().enumerate() {
                collect_added(
                    child,
                    &append_pointer(path, &index.to_string()),
                    sensitive,
                    output,
                );
            }
        }
        _ => push_diff(
            path,
            RawConfigDiffOperation::Add,
            None,
            Some(value),
            sensitive,
            output,
        ),
    }
}

fn collect_removed(
    value: &Value,
    path: &str,
    sensitive: bool,
    output: &mut Vec<RawConfigDiffEntry>,
) {
    match value {
        Value::Object(object) if !object.is_empty() => {
            for (key, child) in object {
                collect_removed(
                    child,
                    &append_pointer(path, key),
                    sensitive || is_sensitive_key(key),
                    output,
                );
            }
        }
        Value::Array(items) if !items.is_empty() => {
            for (index, child) in items.iter().enumerate() {
                collect_removed(
                    child,
                    &append_pointer(path, &index.to_string()),
                    sensitive,
                    output,
                );
            }
        }
        _ => push_diff(
            path,
            RawConfigDiffOperation::Remove,
            Some(value),
            None,
            sensitive,
            output,
        ),
    }
}

fn push_diff(
    path: &str,
    operation: RawConfigDiffOperation,
    before: Option<&Value>,
    after: Option<&Value>,
    sensitive: bool,
    output: &mut Vec<RawConfigDiffEntry>,
) {
    output.push(RawConfigDiffEntry {
        path: path.to_string(),
        operation,
        before_exists: before.is_some(),
        before: before.cloned().unwrap_or(Value::Null),
        after_exists: after.is_some(),
        after: after.cloned().unwrap_or(Value::Null),
        sensitive,
    });
}

fn append_pointer(path: &str, segment: &str) -> String {
    format!("{path}/{}", segment.replace('~', "~0").replace('/', "~1"))
}

fn is_sensitive_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase().replace(['-', '.', ' '], "_");
    matches!(
        normalized.as_str(),
        "key"
            | "token"
            | "secret"
            | "password"
            | "passphrase"
            | "credential"
            | "credentials"
            | "authorization"
            | "cookie"
    ) || normalized.contains("api_key")
        || normalized.contains("apikey")
        || normalized.contains("access_token")
        || normalized.contains("refresh_token")
        || normalized.contains("private_key")
        || normalized.ends_with("_secret")
        || normalized.ends_with("_password")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::{read_manifest, restore_backup_dir_after_gate};
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
        let vault = env::temp_dir().join(format!("raw-plugin-config-{name}-{unique}"));
        let plugin = vault.join(".obsidian/plugins/example");
        fs::create_dir_all(&plugin).expect("fixture dirs");
        fs::write(
            plugin.join("manifest.json"),
            r#"{"id":"example","name":"Example","version":"1.0.0"}"#,
        )
        .expect("manifest");
        fs::write(plugin.join("main.js"), "module.exports = {};").expect("main");
        fs::write(
            vault.join(".obsidian/community-plugins.json"),
            r#"["example"]"#,
        )
        .expect("enabled");
        vault
    }

    fn raw_state(value: Value) -> RawPluginConfiguration {
        let raw_text = serde_json::to_string(&value).unwrap();
        RawPluginConfiguration {
            plugin_id: "example".to_string(),
            exists: true,
            byte_length: raw_text.len(),
            revision: raw_revision(true, raw_text.as_bytes()),
            raw_text,
            value: Some(value),
            parse_error: None,
        }
    }

    fn current_revision(vault: &Path) -> String {
        inspect_raw_plugin_configuration(vault.display().to_string(), "example".to_string())
            .expect("current raw configuration")
            .revision
    }

    #[cfg(windows)]
    fn create_file_link(target: &Path, link: &Path) -> bool {
        std::os::windows::fs::symlink_file(target, link).is_ok()
    }

    #[cfg(unix)]
    fn create_file_link(target: &Path, link: &Path) -> bool {
        std::os::unix::fs::symlink(target, link).is_ok()
    }

    #[test]
    fn recursive_diff_reports_add_change_remove_and_escaped_paths() {
        let current = raw_state(json!({
            "same": true,
            "changed": 1,
            "removed": "old",
            "a/b~c": {"value": 1}
        }));
        let preview = build_raw_diff(
            &current,
            &json!({
                "same": true,
                "changed": 2,
                "added": "new",
                "a/b~c": {"value": 3}
            }),
        );

        assert_eq!(preview.entries.len(), 4);
        assert!(preview.entries.iter().any(|entry| {
            entry.path == "/added" && entry.operation == RawConfigDiffOperation::Add
        }));
        assert!(preview.entries.iter().any(|entry| {
            entry.path == "/changed" && entry.operation == RawConfigDiffOperation::Change
        }));
        assert!(preview.entries.iter().any(|entry| {
            entry.path == "/removed" && entry.operation == RawConfigDiffOperation::Remove
        }));
        assert!(preview
            .entries
            .iter()
            .any(|entry| entry.path == "/a~1b~0c/value"));
    }

    #[test]
    fn secret_like_paths_are_marked_sensitive() {
        let current = raw_state(json!({"auth": {"apiKey": "old"}, "monkey": "old"}));
        let preview = build_raw_diff(
            &current,
            &json!({"auth": {"apiKey": "new"}, "monkey": "new"}),
        );

        assert!(preview
            .entries
            .iter()
            .find(|entry| entry.path == "/auth/apiKey")
            .is_some_and(|entry| entry.sensitive));
        assert!(!preview
            .entries
            .iter()
            .find(|entry| entry.path == "/monkey")
            .is_some_and(|entry| entry.sensitive));
    }

    #[test]
    fn no_op_diff_is_empty() {
        let current = raw_state(json!({"value": [1, 2, 3]}));
        assert!(build_raw_diff(&current, &json!({"value": [1, 2, 3]}))
            .entries
            .is_empty());
    }

    #[test]
    fn inspects_valid_missing_and_malformed_files() {
        let vault = temp_vault("inspect");
        let data_path = vault.join(".obsidian/plugins/example/data.json");

        let missing =
            inspect_raw_plugin_configuration(vault.display().to_string(), "example".to_string())
                .expect("missing state");
        assert!(!missing.exists);
        assert_eq!(missing.value, Some(json!({})));

        fs::write(&data_path, r#"{"nested":{"value":3}}"#).unwrap();
        let valid =
            inspect_raw_plugin_configuration(vault.display().to_string(), "example".to_string())
                .expect("valid state");
        assert_eq!(valid.value, Some(json!({"nested": {"value": 3}})));
        assert!(valid.parse_error.is_none());

        fs::write(&data_path, "{broken").unwrap();
        let malformed =
            inspect_raw_plugin_configuration(vault.display().to_string(), "example".to_string())
                .expect("malformed state");
        assert!(malformed.value.is_none());
        assert!(malformed.parse_error.is_some());
        assert_eq!(malformed.raw_text, "{broken");
        let replacement = preview_raw_plugin_configuration(
            vault.display().to_string(),
            "example".to_string(),
            json!({"repaired": true}),
        )
        .expect("replacement preview");
        assert_eq!(replacement.entries.len(), 1);
        assert!(replacement.entries[0].before_exists);
        assert_eq!(replacement.entries[0].path, "");

        fs::remove_dir_all(vault).expect("cleanup");
    }

    #[test]
    fn raw_confirmation_is_independent_from_the_closed_obsidian_gate() {
        let vault = temp_vault("confirmation");
        let error = save_raw_plugin_configuration(
            vault.display().to_string(),
            "example".to_string(),
            json!({"enabled": true}),
            "not-used".to_string(),
            false,
            false,
        )
        .expect_err("raw confirmation must be required first");
        assert_eq!(error.code, "raw_configuration_confirmation_required");

        let error = save_raw_plugin_configuration(
            vault.display().to_string(),
            "example".to_string(),
            json!({"enabled": true}),
            "not-used".to_string(),
            true,
            false,
        )
        .expect_err("closed Obsidian confirmation must remain required");
        assert_eq!(error.code, "obsidian_not_confirmed_closed");

        fs::remove_dir_all(vault).expect("cleanup");
    }

    #[test]
    fn saves_complete_json_and_rejects_semantic_no_op() {
        let vault = temp_vault("save");
        let data_path = vault.join(".obsidian/plugins/example/data.json");
        fs::write(&data_path, r#"{"keep":1,"remove":2}"#).expect("original config");
        let proposed = json!({"keep": 3, "added": [true, null]});
        let revision = current_revision(&vault);

        let summary = save_raw_plugin_configuration_after_gate(
            vault.display().to_string(),
            "example".to_string(),
            proposed.clone(),
            revision,
        )
        .expect("raw save");
        assert_eq!(summary.results[0].action, "save-plugin-raw-configuration");
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(&data_path).expect("saved bytes"))
                .expect("saved json"),
            proposed
        );
        let manifest = read_manifest(Path::new(&summary.backup_paths[0])).expect("manifest");
        assert_eq!(
            manifest.plugin_context.expect("plugin context").operation,
            "save-raw-configuration"
        );

        let error = save_raw_plugin_configuration_after_gate(
            vault.display().to_string(),
            "example".to_string(),
            proposed,
            current_revision(&vault),
        )
        .expect_err("semantic no-op must be rejected");
        assert_eq!(error.code, "raw_configuration_unchanged");

        fs::remove_dir_all(vault).expect("cleanup");
    }

    #[test]
    fn creates_missing_config_and_restores_malformed_original_bytes() {
        let missing_vault = temp_vault("missing-save");
        let missing_path = missing_vault.join(".obsidian/plugins/example/data.json");
        save_raw_plugin_configuration_after_gate(
            missing_vault.display().to_string(),
            "example".to_string(),
            json!({}),
            current_revision(&missing_vault),
        )
        .expect("create empty config");
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(&missing_path).expect("created config"))
                .expect("created json"),
            json!({})
        );
        fs::remove_dir_all(missing_vault).expect("missing cleanup");

        let vault = temp_vault("malformed-restore");
        let data_path = vault.join(".obsidian/plugins/example/data.json");
        let original = b"{\r\n  broken: true\r\n}\r\n";
        fs::write(&data_path, original).expect("malformed original");
        let revision = current_revision(&vault);
        let summary = save_raw_plugin_configuration_after_gate(
            vault.display().to_string(),
            "example".to_string(),
            json!({"repaired": true}),
            revision,
        )
        .expect("replace malformed config");
        let backup_path = PathBuf::from(&summary.backup_paths[0]);
        let backed_up_data = backup_path.join("files/.obsidian/plugins/example/data.json");
        assert_eq!(fs::read(&backed_up_data).expect("backup bytes"), original);

        restore_backup_dir_after_gate(
            vault.display().to_string(),
            backup_path.display().to_string(),
        )
        .expect("restore plugin backup");
        assert_eq!(fs::read(&data_path).expect("restored bytes"), original);

        fs::remove_dir_all(vault).expect("cleanup");
    }

    #[test]
    fn rejects_a_file_changed_after_diff_preview() {
        let vault = temp_vault("stale-preview");
        let data_path = vault.join(".obsidian/plugins/example/data.json");
        fs::write(&data_path, r#"{"value":1}"#).expect("initial config");
        let preview = preview_raw_plugin_configuration(
            vault.display().to_string(),
            "example".to_string(),
            json!({"value": 2}),
        )
        .expect("preview");
        fs::write(&data_path, r#"{"value":3}"#).expect("external change");

        let error = save_raw_plugin_configuration_after_gate(
            vault.display().to_string(),
            "example".to_string(),
            json!({"value": 2}),
            preview.current_revision,
        )
        .expect_err("stale preview must not overwrite a newer file");
        assert_eq!(error.code, "raw_configuration_changed_since_preview");
        assert_eq!(fs::read_to_string(&data_path).unwrap(), r#"{"value":3}"#);

        fs::remove_dir_all(vault).expect("cleanup");
    }

    #[test]
    fn linked_data_json_is_rejected_when_fixture_is_supported() {
        let vault = temp_vault("linked-data");
        let plugin_dir = vault.join(".obsidian/plugins/example");
        let target = plugin_dir.join("external-config.json");
        let data_path = plugin_dir.join("data.json");
        fs::write(&target, r#"{"value":1}"#).expect("link target");
        if !create_file_link(&target, &data_path) {
            fs::remove_dir_all(vault).expect("cleanup unsupported fixture");
            return;
        }

        let error =
            inspect_raw_plugin_configuration(vault.display().to_string(), "example".to_string())
                .expect_err("linked data.json must be rejected");
        assert_eq!(error.code, "unsupported_link_path");

        fs::remove_file(&data_path).expect("remove link");
        fs::remove_dir_all(vault).expect("cleanup");
    }
}
