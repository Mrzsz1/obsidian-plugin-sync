use crate::errors::{AppError, AppResult};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::{collections::BTreeSet, fs, path::Path};

pub const CONFIG_DIR: &str = ".obsidian";
pub const PLUGINS_DIR: &str = "plugins";
pub const COMMUNITY_PLUGINS_FILE: &str = "community-plugins.json";
pub const APP_JSON_FILE: &str = "app.json";
pub const BACKUP_DIR_NAME: &str = ".obsidian-plugin-sync-backups";
pub const BACKUP_IGNORE_FILTER: &str = ".obsidian-plugin-sync-backups/";

pub fn config_dir(vault_path: &Path) -> std::path::PathBuf {
    vault_path.join(CONFIG_DIR)
}

pub fn plugins_dir(vault_path: &Path) -> std::path::PathBuf {
    config_dir(vault_path).join(PLUGINS_DIR)
}

pub fn community_plugins_path(vault_path: &Path) -> std::path::PathBuf {
    config_dir(vault_path).join(COMMUNITY_PLUGINS_FILE)
}

pub fn app_json_path(vault_path: &Path) -> std::path::PathBuf {
    config_dir(vault_path).join(APP_JSON_FILE)
}

pub fn backup_root(vault_path: &Path) -> std::path::PathBuf {
    vault_path.join(BACKUP_DIR_NAME)
}

pub fn read_enabled_plugin_ids(vault_path: &Path) -> AppResult<Vec<String>> {
    let path = community_plugins_path(vault_path);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content =
        fs::read_to_string(&path).map_err(|error| AppError::from(error).with_path(&path))?;
    let ids: Vec<String> =
        serde_json::from_str(&content).map_err(|error| AppError::from(error).with_path(&path))?;
    Ok(ids)
}

pub fn write_enabled_plugin_ids(vault_path: &Path, ids: &[String]) -> AppResult<()> {
    let path = community_plugins_path(vault_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| AppError::from(error).with_path(parent))?;
    }
    let mut unique = BTreeSet::new();
    for id in ids {
        unique.insert(id.clone());
    }
    let normalized: Vec<String> = unique.into_iter().collect();
    write_json_atomic(&path, &normalized)
}

pub fn write_json_atomic<T: Serialize + ?Sized>(path: &Path, value: &T) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| AppError::from(error).with_path(parent))?;
    }
    let content = serde_json::to_string_pretty(value)
        .map_err(|error| AppError::from(error).with_path(path))?;
    let temp_path = path.with_extension("ops-temp");
    fs::write(&temp_path, content).map_err(|error| AppError::from(error).with_path(&temp_path))?;
    if path.exists() {
        fs::remove_file(path).map_err(|error| AppError::from(error).with_path(path))?;
    }
    fs::rename(&temp_path, path).map_err(|error| AppError::from(error).with_path(path))
}

pub fn ensure_backup_dir_ignored(vault_path: &Path) -> AppResult<bool> {
    let path = app_json_path(vault_path);
    let mut value = if path.exists() {
        let content =
            fs::read_to_string(&path).map_err(|error| AppError::from(error).with_path(&path))?;
        serde_json::from_str::<Value>(&content)
            .map_err(|error| AppError::from(error).with_path(&path))?
    } else {
        json!({})
    };

    if !value.is_object() {
        value = json!({});
    }

    let object = value.as_object_mut().ok_or_else(|| {
        AppError::new("invalid_app_json", "app.json 根节点不是对象").with_path(&path)
    })?;

    let changed = add_ignore_filter(object);
    if changed {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| AppError::from(error).with_path(parent))?;
        }
        let content = serde_json::to_string_pretty(&value)
            .map_err(|error| AppError::from(error).with_path(&path))?;
        fs::write(&path, content).map_err(|error| AppError::from(error).with_path(&path))?;
    }
    Ok(changed)
}

fn add_ignore_filter(object: &mut Map<String, Value>) -> bool {
    let entry = object
        .entry("userIgnoreFilters".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    if !entry.is_array() {
        *entry = Value::Array(Vec::new());
    }
    let filters = entry.as_array_mut().expect("array ensured above");
    let already_exists = filters
        .iter()
        .any(|value| value.as_str() == Some(BACKUP_IGNORE_FILTER));
    if already_exists {
        return false;
    }
    filters.push(Value::String(BACKUP_IGNORE_FILTER.to_string()));
    true
}
