mod app_config;
mod backup;
mod diff;
mod errors;
mod fs_safety;
mod models;
mod obsidian_config;
mod plugin_adapters;
mod plugin_manager;
mod plugin_settings;
mod process;
mod raw_plugin_config;
mod reports;
mod settings_bridge;
mod sync;
mod vault;
mod versions;

use crate::{
    app_config::{load_settings, save_settings},
    backup::{list_backup_infos, restore_backup_dir},
    diff::build_diff_for_targets,
    errors::AppResult,
    models::{
        AppSettings, BackupInfo, LocalPluginInstallPreview, ManagedPluginSettings,
        PluginAdapterSettingChange, RawConfigDiffPreview, RawPluginConfiguration,
        SettingsBridgeRequestOperation, SettingsBridgeStatus, SyncPlan, SyncSummary, TargetDiff,
        Vault, VaultInventory, VaultPluginManagementInventory,
    },
    plugin_manager::{
        delete_plugin as delete_managed_plugin_impl, inspect_local_plugin,
        install_plugin_from_local_folder, open_plugin_folder, save_plugin_adapter_configuration,
        save_plugin_configuration, scan_plugin_management_inventory, set_plugin_enabled,
    },
    plugin_settings::inspect_plugin_settings,
    process::obsidian_is_running,
    raw_plugin_config::{inspect_raw_plugin_configuration, preview_raw_plugin_configuration},
    settings_bridge::{
        inspect_settings_bridge, install_settings_bridge, launch_settings_bridge_request,
        remove_settings_bridge, set_settings_bridge_enabled,
    },
    sync::apply_plan,
    vault::{discover_registered_vaults, scan_vault_inventory, validate_vault},
};
use tauri::{
    menu::MenuBuilder,
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager,
};

const TRAY_SHOW_ID: &str = "show";
const TRAY_QUIT_ID: &str = "quit";

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

#[tauri::command]
fn load_app_settings() -> AppResult<AppSettings> {
    load_settings()
}

#[tauri::command]
fn save_app_settings(settings: AppSettings) -> AppResult<()> {
    save_settings(&settings)
}

#[tauri::command]
fn discover_vaults() -> AppResult<Vec<Vault>> {
    discover_registered_vaults()
}

#[tauri::command]
fn validate_vault_path(path: String) -> AppResult<Vault> {
    validate_vault(path, crate::models::VaultSource::Manual)
}

#[tauri::command]
fn scan_vault(path: String) -> AppResult<VaultInventory> {
    scan_vault_inventory(path)
}

#[tauri::command]
fn scan_managed_plugins(vault_path: String) -> AppResult<VaultPluginManagementInventory> {
    scan_plugin_management_inventory(vault_path)
}

#[tauri::command]
async fn inspect_managed_plugin_settings(
    vault_path: String,
    plugin_id: String,
) -> AppResult<ManagedPluginSettings> {
    tauri::async_runtime::spawn_blocking(move || inspect_plugin_settings(vault_path, plugin_id))
        .await
        .map_err(|error| {
            crate::errors::AppError::new(
                "settings_inference_task_failed",
                "插件设置推断任务执行失败",
            )
            .with_details(error.to_string())
        })?
}

#[tauri::command]
fn inspect_local_plugin_folder(
    vault_path: String,
    source_folder_path: String,
) -> AppResult<LocalPluginInstallPreview> {
    inspect_local_plugin(vault_path, source_folder_path)
}

#[tauri::command]
fn set_managed_plugin_enabled(
    vault_path: String,
    plugin_id: String,
    enabled: bool,
    obsidian_closed_confirmed: bool,
) -> AppResult<SyncSummary> {
    set_plugin_enabled(vault_path, plugin_id, enabled, obsidian_closed_confirmed)
}

#[tauri::command]
fn save_managed_plugin_configuration(
    vault_path: String,
    plugin_id: String,
    configuration: serde_json::Value,
    obsidian_closed_confirmed: bool,
    risk_override_confirmed: bool,
) -> AppResult<SyncSummary> {
    save_plugin_configuration(
        vault_path,
        plugin_id,
        configuration,
        obsidian_closed_confirmed,
        risk_override_confirmed,
    )
}

#[tauri::command]
fn save_managed_plugin_adapter_configuration(
    vault_path: String,
    plugin_id: String,
    adapter_id: String,
    changes: Vec<PluginAdapterSettingChange>,
    obsidian_closed_confirmed: bool,
) -> AppResult<SyncSummary> {
    save_plugin_adapter_configuration(
        vault_path,
        plugin_id,
        adapter_id,
        changes,
        obsidian_closed_confirmed,
    )
}

#[tauri::command]
fn inspect_raw_managed_plugin_configuration(
    vault_path: String,
    plugin_id: String,
) -> AppResult<RawPluginConfiguration> {
    inspect_raw_plugin_configuration(vault_path, plugin_id)
}

#[tauri::command]
fn preview_raw_managed_plugin_configuration(
    vault_path: String,
    plugin_id: String,
    proposed: serde_json::Value,
) -> AppResult<RawConfigDiffPreview> {
    preview_raw_plugin_configuration(vault_path, plugin_id, proposed)
}

#[tauri::command]
fn save_raw_managed_plugin_configuration(
    vault_path: String,
    plugin_id: String,
    proposed: serde_json::Value,
    expected_current_revision: String,
    raw_risk_confirmed: bool,
    obsidian_closed_confirmed: bool,
) -> AppResult<SyncSummary> {
    crate::raw_plugin_config::save_raw_plugin_configuration(
        vault_path,
        plugin_id,
        proposed,
        expected_current_revision,
        raw_risk_confirmed,
        obsidian_closed_confirmed,
    )
}

#[tauri::command]
fn inspect_managed_settings_bridge(
    vault_path: String,
    plugin_id: String,
) -> AppResult<SettingsBridgeStatus> {
    inspect_settings_bridge(vault_path, plugin_id)
}

#[tauri::command]
fn install_managed_settings_bridge(
    vault_path: String,
    enable_after_install: bool,
    allow_downgrade: bool,
    obsidian_closed_confirmed: bool,
) -> AppResult<SyncSummary> {
    install_settings_bridge(
        vault_path,
        enable_after_install,
        allow_downgrade,
        obsidian_closed_confirmed,
    )
}

#[tauri::command]
fn set_managed_settings_bridge_enabled(
    vault_path: String,
    enabled: bool,
    obsidian_closed_confirmed: bool,
) -> AppResult<SyncSummary> {
    set_settings_bridge_enabled(vault_path, enabled, obsidian_closed_confirmed)
}

#[tauri::command]
fn remove_managed_settings_bridge(
    vault_path: String,
    remove_confirmed: bool,
    obsidian_closed_confirmed: bool,
) -> AppResult<SyncSummary> {
    remove_settings_bridge(vault_path, remove_confirmed, obsidian_closed_confirmed)
}

#[tauri::command]
fn launch_managed_settings_bridge_request(
    vault_path: String,
    plugin_id: String,
    operation: SettingsBridgeRequestOperation,
) -> AppResult<()> {
    launch_settings_bridge_request(vault_path, plugin_id, operation)
}

#[tauri::command]
fn install_local_plugin(
    vault_path: String,
    source_folder_path: String,
    overwrite_existing: bool,
    obsidian_closed_confirmed: bool,
) -> AppResult<SyncSummary> {
    install_plugin_from_local_folder(
        vault_path,
        source_folder_path,
        overwrite_existing,
        obsidian_closed_confirmed,
    )
}

#[tauri::command]
fn delete_managed_plugin(
    vault_path: String,
    plugin_id: String,
    delete_confirmed: bool,
    secondary_confirmed: bool,
    obsidian_closed_confirmed: bool,
) -> AppResult<SyncSummary> {
    delete_managed_plugin_impl(
        vault_path,
        plugin_id,
        delete_confirmed,
        secondary_confirmed,
        obsidian_closed_confirmed,
    )
}

#[tauri::command]
fn open_managed_plugin_folder(vault_path: String, plugin_id: String) -> AppResult<()> {
    open_plugin_folder(vault_path, plugin_id)
}

#[tauri::command]
fn build_vault_diff(
    source_vault_path: String,
    target_vault_paths: Vec<String>,
) -> AppResult<Vec<TargetDiff>> {
    build_diff_for_targets(source_vault_path, target_vault_paths)
}

#[tauri::command]
fn check_obsidian_running() -> AppResult<bool> {
    obsidian_is_running()
}

#[tauri::command]
fn apply_sync_plan(plan: SyncPlan) -> AppResult<SyncSummary> {
    apply_plan(plan)
}

#[tauri::command]
fn list_backups(vault_path: String) -> AppResult<Vec<BackupInfo>> {
    list_backup_infos(vault_path)
}

#[tauri::command]
fn restore_backup(
    vault_path: String,
    backup_path: String,
    obsidian_closed_confirmed: bool,
) -> AppResult<SyncSummary> {
    restore_backup_dir(vault_path, backup_path, obsidian_closed_confirmed)
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let menu = MenuBuilder::new(app)
                .text(TRAY_SHOW_ID, "显示窗口")
                .separator()
                .text(TRAY_QUIT_ID, "退出")
                .build()?;

            let mut tray_builder = TrayIconBuilder::new()
                .tooltip("Obsidian Plugin Sync")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_tray_icon_event(|tray, event| match event {
                    TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    }
                    | TrayIconEvent::DoubleClick {
                        button: MouseButton::Left,
                        ..
                    } => show_main_window(tray.app_handle()),
                    _ => {}
                })
                .on_menu_event(|app, event| match event.id().as_ref() {
                    TRAY_SHOW_ID => show_main_window(app),
                    TRAY_QUIT_ID => app.exit(0),
                    _ => {}
                });

            if let Some(icon) = app.default_window_icon() {
                tray_builder = tray_builder.icon(icon.clone());
            }

            tray_builder.build(app)?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            load_app_settings,
            save_app_settings,
            discover_vaults,
            validate_vault_path,
            scan_vault,
            scan_managed_plugins,
            inspect_managed_plugin_settings,
            inspect_local_plugin_folder,
            set_managed_plugin_enabled,
            save_managed_plugin_configuration,
            save_managed_plugin_adapter_configuration,
            inspect_raw_managed_plugin_configuration,
            preview_raw_managed_plugin_configuration,
            save_raw_managed_plugin_configuration,
            inspect_managed_settings_bridge,
            install_managed_settings_bridge,
            set_managed_settings_bridge_enabled,
            remove_managed_settings_bridge,
            launch_managed_settings_bridge_request,
            install_local_plugin,
            delete_managed_plugin,
            open_managed_plugin_folder,
            build_vault_diff,
            check_obsidian_running,
            apply_sync_plan,
            list_backups,
            restore_backup
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Obsidian Plugin Sync");
}
