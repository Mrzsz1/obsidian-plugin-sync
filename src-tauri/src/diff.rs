use crate::{
    errors::{AppError, AppResult},
    fs_safety,
    models::{PluginDiff, PluginDiffChecks, PluginDiffStatus, PluginInventoryItem, TargetDiff},
    plugin_adapters::{configurations_equal_for_sync, should_filter_configuration_for_sync},
    vault::scan_vault_inventory,
    versions::compare_versions,
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    path::{Path, PathBuf},
};

pub fn build_diff_for_targets(
    source_vault_path: String,
    target_vault_paths: Vec<String>,
) -> AppResult<Vec<TargetDiff>> {
    let source_inventory = scan_vault_inventory(source_vault_path)?;
    let source_plugins = valid_plugin_map(&source_inventory.plugins);
    let mut diffs = Vec::new();

    for target_path in target_vault_paths {
        let target_inventory = scan_vault_inventory(target_path)?;
        let target_plugins = valid_plugin_map(&target_inventory.plugins);
        let mut plugin_ids = BTreeSet::new();
        plugin_ids.extend(source_plugins.keys().cloned());
        plugin_ids.extend(target_plugins.keys().cloned());

        let plugins = plugin_ids
            .into_iter()
            .map(|plugin_id| {
                let source_plugin = source_plugins.get(&plugin_id).cloned();
                let target_plugin = target_plugins.get(&plugin_id).cloned();
                build_plugin_diff(plugin_id, source_plugin, target_plugin)
            })
            .collect();

        diffs.push(TargetDiff {
            target_vault: target_inventory.vault,
            plugins,
            warnings: target_inventory.warnings,
        });
    }

    Ok(diffs)
}

fn valid_plugin_map(plugins: &[PluginInventoryItem]) -> HashMap<String, PluginInventoryItem> {
    plugins
        .iter()
        .filter_map(|plugin| plugin.id.as_ref().map(|id| (id.clone(), plugin.clone())))
        .collect()
}

fn build_plugin_diff(
    plugin_id: String,
    source_plugin: Option<PluginInventoryItem>,
    target_plugin: Option<PluginInventoryItem>,
) -> PluginDiff {
    let display_name = source_plugin
        .as_ref()
        .and_then(|plugin| plugin.name.clone())
        .or_else(|| {
            target_plugin
                .as_ref()
                .and_then(|plugin| plugin.name.clone())
        })
        .unwrap_or_else(|| plugin_id.clone());

    let status = match (&source_plugin, &target_plugin) {
        (Some(source), Some(target)) if !source.valid || !target.valid => PluginDiffStatus::Invalid,
        (Some(source), Some(target))
            if source.unsupported_reason.is_some() || target.unsupported_reason.is_some() =>
        {
            PluginDiffStatus::Unsupported
        }
        (Some(source), Some(target)) => {
            compare_versions(source.version.as_deref(), target.version.as_deref())
        }
        (Some(_), None) => PluginDiffStatus::MissingInTarget,
        (None, Some(_)) => PluginDiffStatus::TargetOnly,
        (None, None) => PluginDiffStatus::Invalid,
    };

    let mut warnings = Vec::new();
    if status == PluginDiffStatus::SourceOlder {
        warnings.push("源库版本低于目标库，默认不选择降级".to_string());
    }
    if status == PluginDiffStatus::TargetOnly {
        warnings.push("目标库独有插件，删除前会二次确认并备份".to_string());
    }

    let plugin_files_equal = match plugin_files_equal(&source_plugin, &target_plugin) {
        Ok(equal) => equal,
        Err(error) => {
            warnings.push(format!("无法比较插件本体：{}", error.message));
            false
        }
    };
    let data_json_equal = match data_json_equal(&source_plugin, &target_plugin) {
        Ok(equal) => equal,
        Err(error) => {
            warnings.push(format!("无法比较 data.json：{}", error.message));
            false
        }
    };
    let enabled_state_equal = match (&source_plugin, &target_plugin) {
        (Some(source), Some(target)) => source.enabled == target.enabled,
        (None, None) => true,
        _ => false,
    };

    PluginDiff {
        plugin_id,
        display_name,
        status,
        checks: PluginDiffChecks {
            plugin_files_equal,
            data_json_equal,
            enabled_state_equal,
        },
        source_plugin,
        target_plugin,
        warnings,
    }
}

fn plugin_files_equal(
    source_plugin: &Option<PluginInventoryItem>,
    target_plugin: &Option<PluginInventoryItem>,
) -> AppResult<bool> {
    let (Some(source), Some(target)) = (source_plugin, target_plugin) else {
        return Ok(source_plugin.is_none() && target_plugin.is_none());
    };

    let source_files = plugin_file_snapshot(Path::new(&source.folder_path))?;
    let target_files = plugin_file_snapshot(Path::new(&target.folder_path))?;
    Ok(source_files == target_files)
}

fn plugin_file_snapshot(root: &Path) -> AppResult<BTreeMap<String, Vec<u8>>> {
    let mut output = BTreeMap::new();
    collect_plugin_files(root, root, &mut output)?;
    Ok(output)
}

fn collect_plugin_files(
    root: &Path,
    current: &Path,
    output: &mut BTreeMap<String, Vec<u8>>,
) -> AppResult<()> {
    if fs_safety::is_link_path(current)? {
        return Err(
            AppError::new("unsupported_link_path", "不支持比较链接目录或链接文件")
                .with_path(current),
        );
    }

    let metadata =
        fs::metadata(current).map_err(|error| AppError::from(error).with_path(current))?;
    if metadata.is_dir() {
        for entry in
            fs::read_dir(current).map_err(|error| AppError::from(error).with_path(current))?
        {
            let entry = entry.map_err(|error| AppError::from(error).with_path(current))?;
            collect_plugin_files(root, &entry.path(), output)?;
        }
        return Ok(());
    }

    let relative_path = current.strip_prefix(root).map_err(|error| {
        AppError::new("invalid_plugin_file_path", error.to_string()).with_path(current)
    })?;
    if relative_path == Path::new("data.json") {
        return Ok(());
    }

    let key = normalized_relative_path(relative_path);
    let content = fs::read(current).map_err(|error| AppError::from(error).with_path(current))?;
    output.insert(key, content);
    Ok(())
}

fn data_json_equal(
    source_plugin: &Option<PluginInventoryItem>,
    target_plugin: &Option<PluginInventoryItem>,
) -> AppResult<bool> {
    let (Some(source), Some(target)) = (source_plugin, target_plugin) else {
        return Ok(source_plugin.is_none() && target_plugin.is_none());
    };

    let source_data = PathBuf::from(&source.folder_path).join("data.json");
    let target_data = PathBuf::from(&target.folder_path).join("data.json");
    match (source_data.exists(), target_data.exists()) {
        (false, false) => Ok(true),
        (true, true) => adapter_aware_json_file_equal(source, target, &source_data, &target_data),
        _ => Ok(false),
    }
}

fn adapter_aware_json_file_equal(
    source_plugin: &PluginInventoryItem,
    target_plugin: &PluginInventoryItem,
    source: &Path,
    target: &Path,
) -> AppResult<bool> {
    let source_bytes = fs::read(source).map_err(|error| AppError::from(error).with_path(source))?;
    let target_bytes = fs::read(target).map_err(|error| AppError::from(error).with_path(target))?;
    let source_json = serde_json::from_slice::<Value>(&source_bytes);
    let target_json = serde_json::from_slice::<Value>(&target_bytes);

    match (source_json, target_json) {
        (Ok(source_value), Ok(target_value)) => {
            if let Some(equal) = configurations_equal_for_sync(
                source_plugin,
                target_plugin,
                &source_value,
                &target_value,
            )? {
                return Ok(equal);
            }
            Ok(source_value == target_value)
        }
        (source_result, target_result) => {
            if should_filter_configuration_for_sync(source_plugin, Some(target_plugin), false) {
                return Err(AppError::new(
                    "plugin_adapter_sync_configuration_invalid",
                    "适配插件的 data.json 无法解析，不能安全忽略设备本地设置",
                )
                .with_details(format!(
                    "source: {}; target: {}",
                    source_result
                        .err()
                        .map(|error| error.to_string())
                        .unwrap_or_else(|| "valid".to_string()),
                    target_result
                        .err()
                        .map(|error| error.to_string())
                        .unwrap_or_else(|| "valid".to_string())
                )));
            }
            Ok(source_bytes == target_bytes)
        }
    }
}

fn normalized_relative_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::UnsupportedReason;
    use std::{
        env,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn plugin_file_compare_ignores_root_data_json() {
        let root = temp_root("plugin-file-compare");
        let source = root.join("source");
        let target = root.join("target");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&target).unwrap();
        fs::write(source.join("main.js"), "console.log('same')").unwrap();
        fs::write(target.join("main.js"), "console.log('same')").unwrap();
        fs::write(source.join("data.json"), "{\"value\":1}").unwrap();
        fs::write(target.join("data.json"), "{\"value\":2}").unwrap();

        assert_eq!(
            plugin_file_snapshot(&source).unwrap(),
            plugin_file_snapshot(&target).unwrap()
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn data_json_compare_uses_json_value_equality() {
        let root = temp_root("data-json-compare");
        fs::create_dir_all(&root).unwrap();
        let source = root.join("source.json");
        let target = root.join("target.json");
        fs::write(&source, "{\"a\":1,\"b\":2}").unwrap();
        fs::write(&target, "{\n  \"b\": 2,\n  \"a\": 1\n}").unwrap();

        let source_plugin = plugin("example", "1.0.0");
        let target_plugin = plugin("example", "1.0.0");
        assert!(
            adapter_aware_json_file_equal(&source_plugin, &target_plugin, &source, &target,)
                .unwrap()
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn adapted_data_comparison_ignores_device_local_paths() {
        let root = temp_root("adapted-data-json-compare");
        fs::create_dir_all(&root).unwrap();
        let source = root.join("source.json");
        let target = root.join("target.json");
        fs::write(
            &source,
            r#"{"locale":"zh-cn","providerConfigs":{"claude":{"cliPathsByHost":{"device:source":"C:/source.exe"}}}}"#,
        )
        .unwrap();
        fs::write(
            &target,
            r#"{"locale":"zh-cn","providerConfigs":{"claude":{"cliPathsByHost":{"device:target":"D:/target.exe"}}}}"#,
        )
        .unwrap();
        let source_plugin = plugin("realclaudian", "2.0.24");
        let target_plugin = plugin("realclaudian", "2.0.21");

        assert!(
            adapter_aware_json_file_equal(&source_plugin, &target_plugin, &source, &target,)
                .unwrap()
        );

        let _ = fs::remove_dir_all(root);
    }

    fn plugin(id: &str, version: &str) -> PluginInventoryItem {
        PluginInventoryItem {
            id: Some(id.to_string()),
            folder_name: id.to_string(),
            folder_path: format!("C:/vault/.obsidian/plugins/{id}"),
            manifest_path: format!("C:/vault/.obsidian/plugins/{id}/manifest.json"),
            name: Some(id.to_string()),
            version: Some(version.to_string()),
            enabled: true,
            has_data_json: true,
            valid: true,
            unsupported_reason: None::<UnsupportedReason>,
            warnings: Vec::new(),
        }
    }

    fn temp_root(name: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir().join(format!("ops-{name}-{}-{suffix}", std::process::id()))
    }
}
