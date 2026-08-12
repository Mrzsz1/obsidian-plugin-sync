use crate::{
    errors::{AppError, AppResult},
    fs_safety,
    models::{PluginInventoryItem, UnsupportedReason, Vault, VaultInventory, VaultSource},
    obsidian_config,
};
use serde_json::Value;
use std::{
    collections::{BTreeSet, HashSet},
    env, fs,
    path::{Path, PathBuf},
};

pub fn validate_vault(path: impl Into<String>, source: VaultSource) -> AppResult<Vault> {
    let raw_path = PathBuf::from(path.into());
    let vault_path =
        fs::canonicalize(&raw_path).map_err(|error| AppError::from(error).with_path(&raw_path))?;
    let config_dir = obsidian_config::config_dir(&vault_path);
    if !config_dir.is_dir() {
        return Err(
            AppError::new("invalid_vault", "目录下没有 .obsidian 文件夹").with_path(&vault_path),
        );
    }

    let name = vault_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Obsidian Vault")
        .to_string();

    Ok(Vault {
        id: stable_vault_id(&vault_path),
        name,
        path: vault_path.display().to_string(),
        config_dir: config_dir.display().to_string(),
        source,
        valid: true,
        warnings: Vec::new(),
    })
}

pub fn scan_vault_inventory(path: impl Into<String>) -> AppResult<VaultInventory> {
    let vault = validate_vault(path.into(), VaultSource::Manual)?;
    let vault_path = PathBuf::from(&vault.path);
    let enabled_plugin_ids = obsidian_config::read_enabled_plugin_ids(&vault_path)?;
    let enabled_set: HashSet<String> = enabled_plugin_ids.iter().cloned().collect();
    let plugins_path = obsidian_config::plugins_dir(&vault_path);
    let mut warnings = Vec::new();
    let mut plugins = Vec::new();

    if !plugins_path.exists() {
        return Ok(VaultInventory {
            vault,
            plugins,
            enabled_plugin_ids,
            warnings,
        });
    }

    if fs_safety::is_link_path(&plugins_path)? {
        warnings.push(format!(
            "插件目录是链接目录，已跳过：{}",
            plugins_path.display()
        ));
        return Ok(VaultInventory {
            vault,
            plugins,
            enabled_plugin_ids,
            warnings,
        });
    }

    for entry in fs::read_dir(&plugins_path)
        .map_err(|error| AppError::from(error).with_path(&plugins_path))?
    {
        let entry = entry.map_err(|error| AppError::from(error).with_path(&plugins_path))?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        plugins.push(scan_plugin_folder(&path, &enabled_set)?);
    }

    plugins.sort_by(|left, right| {
        left.name
            .as_deref()
            .unwrap_or(&left.folder_name)
            .cmp(right.name.as_deref().unwrap_or(&right.folder_name))
    });

    Ok(VaultInventory {
        vault,
        plugins,
        enabled_plugin_ids,
        warnings,
    })
}

pub fn discover_registered_vaults() -> AppResult<Vec<Vault>> {
    let Some(appdata) = env::var_os("APPDATA").map(PathBuf::from) else {
        return Ok(Vec::new());
    };
    let obsidian_json = appdata.join("Obsidian").join("obsidian.json");
    if !obsidian_json.exists() {
        return Ok(Vec::new());
    }

    let content = fs::read_to_string(&obsidian_json)
        .map_err(|error| AppError::from(error).with_path(&obsidian_json))?;
    let value: Value = serde_json::from_str(&content)
        .map_err(|error| AppError::from(error).with_path(&obsidian_json))?;
    let mut candidates = BTreeSet::new();
    collect_path_candidates(&value, &mut candidates);

    let mut vaults = Vec::new();
    for candidate in candidates {
        if let Ok(vault) = validate_vault(candidate, VaultSource::ObsidianConfig) {
            vaults.push(vault);
        }
    }
    vaults.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(vaults)
}

fn scan_plugin_folder(
    path: &Path,
    enabled_set: &HashSet<String>,
) -> AppResult<PluginInventoryItem> {
    let folder_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_string();
    let manifest_path = path.join("manifest.json");
    let has_data_json = path.join("data.json").exists();

    if fs_safety::is_link_path(path)? {
        return Ok(invalid_plugin(
            path,
            &manifest_path,
            folder_name,
            has_data_json,
            UnsupportedReason::LinkDirectory,
            "插件目录是链接目录，已跳过",
        ));
    }

    if !manifest_path.exists() {
        return Ok(invalid_plugin(
            path,
            &manifest_path,
            folder_name,
            has_data_json,
            UnsupportedReason::MissingManifest,
            "缺少 manifest.json",
        ));
    }

    let content = match fs::read_to_string(&manifest_path) {
        Ok(content) => content,
        Err(error) => {
            return Ok(invalid_plugin(
                path,
                &manifest_path,
                folder_name,
                has_data_json,
                UnsupportedReason::MalformedManifest,
                &format!("无法读取 manifest.json：{error}"),
            ))
        }
    };

    let value: Value = match serde_json::from_str(&content) {
        Ok(value) => value,
        Err(error) => {
            return Ok(invalid_plugin(
                path,
                &manifest_path,
                folder_name,
                has_data_json,
                UnsupportedReason::MalformedManifest,
                &format!("manifest.json 不是有效 JSON：{error}"),
            ))
        }
    };

    let id = value.get("id").and_then(Value::as_str).map(str::to_string);
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_string);
    let version = value
        .get("version")
        .and_then(Value::as_str)
        .map(str::to_string);

    let Some(id) = id else {
        return Ok(invalid_plugin(
            path,
            &manifest_path,
            folder_name,
            has_data_json,
            UnsupportedReason::MissingId,
            "manifest.json 缺少 id",
        ));
    };

    Ok(PluginInventoryItem {
        enabled: enabled_set.contains(&id),
        id: Some(id),
        folder_name,
        folder_path: path.display().to_string(),
        manifest_path: manifest_path.display().to_string(),
        name,
        version,
        has_data_json,
        valid: true,
        unsupported_reason: None,
        warnings: Vec::new(),
    })
}

fn invalid_plugin(
    folder_path: &Path,
    manifest_path: &Path,
    folder_name: String,
    has_data_json: bool,
    reason: UnsupportedReason,
    warning: &str,
) -> PluginInventoryItem {
    PluginInventoryItem {
        id: None,
        folder_name,
        folder_path: folder_path.display().to_string(),
        manifest_path: manifest_path.display().to_string(),
        name: None,
        version: None,
        enabled: false,
        has_data_json,
        valid: false,
        unsupported_reason: Some(reason),
        warnings: vec![warning.to_string()],
    }
}

fn stable_vault_id(path: &Path) -> String {
    path.display()
        .to_string()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

fn collect_path_candidates(value: &Value, output: &mut BTreeSet<String>) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                if key.eq_ignore_ascii_case("path") {
                    if let Some(path) = child.as_str() {
                        output.insert(path.to_string());
                    }
                }
                collect_path_candidates(child, output);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_path_candidates(item, output);
            }
        }
        Value::String(path) => {
            let candidate = PathBuf::from(path);
            if candidate.join(obsidian_config::CONFIG_DIR).is_dir() {
                output.insert(path.to_string());
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_path_fields_from_unknown_schema() {
        let value = serde_json::json!({
            "vaults": {
                "one": { "path": "C:/Notes/Main" },
                "two": { "other": "ignored" }
            }
        });
        let mut paths = BTreeSet::new();
        collect_path_candidates(&value, &mut paths);
        assert!(paths.contains("C:/Notes/Main"));
    }
}
