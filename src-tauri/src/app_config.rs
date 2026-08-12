use crate::{
    errors::{AppError, AppResult},
    models::AppSettings,
};
use std::{
    env, fs,
    path::{Path, PathBuf},
};

const APP_DIR_NAME: &str = "Obsidian Plugin Sync";
const SETTINGS_FILE: &str = "settings.json";

pub fn app_config_dir() -> AppResult<PathBuf> {
    let appdata = env::var_os("APPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| AppError::new("missing_appdata", "无法读取 Windows APPDATA 目录"))?;
    Ok(appdata.join(APP_DIR_NAME))
}

fn settings_path() -> AppResult<PathBuf> {
    Ok(app_config_dir()?.join(SETTINGS_FILE))
}

pub fn load_settings() -> AppResult<AppSettings> {
    let path = settings_path()?;
    if !path.exists() {
        return Ok(AppSettings::default());
    }
    let content =
        fs::read_to_string(&path).map_err(|error| AppError::from(error).with_path(&path))?;
    serde_json::from_str(&content).map_err(|error| AppError::from(error).with_path(&path))
}

pub fn save_settings(settings: &AppSettings) -> AppResult<()> {
    let dir = app_config_dir()?;
    fs::create_dir_all(&dir).map_err(|error| AppError::from(error).with_path(&dir))?;
    let path = dir.join(SETTINGS_FILE);
    let content = serde_json::to_string_pretty(settings)
        .map_err(|error| AppError::from(error).with_path(&path))?;
    write_json_file(&path, &content)
}

fn write_json_file(path: &Path, content: &str) -> AppResult<()> {
    fs::write(path, content).map_err(|error| AppError::from(error).with_path(path))
}
