import { invoke } from "@tauri-apps/api/core";
import type {
  AppSettings,
  BackupInfo,
  JsonValue,
  LocalPluginInstallPreview,
  ManagedPluginSettings,
  PluginAdapterSettingChange,
  RawConfigDiffPreview,
  RawPluginConfiguration,
  SettingsBridgeRequestOperation,
  SettingsBridgeStatus,
  SyncPlan,
  SyncSummary,
  TargetDiff,
  Vault,
  VaultInventory,
  VaultPluginManagementInventory,
} from "./types";

export const api = {
  loadAppSettings: () => invoke<AppSettings>("load_app_settings"),
  saveAppSettings: (settings: AppSettings) => invoke<void>("save_app_settings", { settings }),
  discoverVaults: () => invoke<Vault[]>("discover_vaults"),
  validateVaultPath: (path: string) => invoke<Vault>("validate_vault_path", { path }),
  scanVault: (path: string) => invoke<VaultInventory>("scan_vault", { path }),
  scanManagedPlugins: (vaultPath: string) =>
    invoke<VaultPluginManagementInventory>("scan_managed_plugins", { vaultPath }),
  inspectManagedPluginSettings: (vaultPath: string, pluginId: string) =>
    invoke<ManagedPluginSettings>("inspect_managed_plugin_settings", { vaultPath, pluginId }),
  inspectLocalPluginFolder: (vaultPath: string, sourceFolderPath: string) =>
    invoke<LocalPluginInstallPreview>("inspect_local_plugin_folder", {
      vaultPath,
      sourceFolderPath,
    }),
  setManagedPluginEnabled: (
    vaultPath: string,
    pluginId: string,
    enabled: boolean,
    obsidianClosedConfirmed: boolean,
  ) =>
    invoke<SyncSummary>("set_managed_plugin_enabled", {
      vaultPath,
      pluginId,
      enabled,
      obsidianClosedConfirmed,
    }),
  saveManagedPluginConfiguration: (
    vaultPath: string,
    pluginId: string,
    configuration: JsonValue,
    obsidianClosedConfirmed: boolean,
    riskOverrideConfirmed: boolean,
  ) =>
    invoke<SyncSummary>("save_managed_plugin_configuration", {
      vaultPath,
      pluginId,
      configuration,
      obsidianClosedConfirmed,
      riskOverrideConfirmed,
    }),
  saveManagedPluginAdapterConfiguration: (
    vaultPath: string,
    pluginId: string,
    adapterId: string,
    changes: PluginAdapterSettingChange[],
    obsidianClosedConfirmed: boolean,
  ) =>
    invoke<SyncSummary>("save_managed_plugin_adapter_configuration", {
      vaultPath,
      pluginId,
      adapterId,
      changes,
      obsidianClosedConfirmed,
    }),
  inspectRawManagedPluginConfiguration: (vaultPath: string, pluginId: string) =>
    invoke<RawPluginConfiguration>("inspect_raw_managed_plugin_configuration", {
      vaultPath,
      pluginId,
    }),
  previewRawManagedPluginConfiguration: (
    vaultPath: string,
    pluginId: string,
    proposed: JsonValue,
  ) =>
    invoke<RawConfigDiffPreview>("preview_raw_managed_plugin_configuration", {
      vaultPath,
      pluginId,
      proposed,
    }),
  saveRawManagedPluginConfiguration: (
    vaultPath: string,
    pluginId: string,
    proposed: JsonValue,
    expectedCurrentRevision: string,
    rawRiskConfirmed: boolean,
    obsidianClosedConfirmed: boolean,
  ) =>
    invoke<SyncSummary>("save_raw_managed_plugin_configuration", {
      vaultPath,
      pluginId,
      proposed,
      expectedCurrentRevision,
      rawRiskConfirmed,
      obsidianClosedConfirmed,
    }),
  inspectManagedSettingsBridge: (vaultPath: string, pluginId: string) =>
    invoke<SettingsBridgeStatus>("inspect_managed_settings_bridge", { vaultPath, pluginId }),
  installManagedSettingsBridge: (
    vaultPath: string,
    enableAfterInstall: boolean,
    allowDowngrade: boolean,
    obsidianClosedConfirmed: boolean,
  ) =>
    invoke<SyncSummary>("install_managed_settings_bridge", {
      vaultPath,
      enableAfterInstall,
      allowDowngrade,
      obsidianClosedConfirmed,
    }),
  setManagedSettingsBridgeEnabled: (
    vaultPath: string,
    enabled: boolean,
    obsidianClosedConfirmed: boolean,
  ) =>
    invoke<SyncSummary>("set_managed_settings_bridge_enabled", {
      vaultPath,
      enabled,
      obsidianClosedConfirmed,
    }),
  removeManagedSettingsBridge: (
    vaultPath: string,
    removeConfirmed: boolean,
    obsidianClosedConfirmed: boolean,
  ) =>
    invoke<SyncSummary>("remove_managed_settings_bridge", {
      vaultPath,
      removeConfirmed,
      obsidianClosedConfirmed,
    }),
  launchManagedSettingsBridgeRequest: (
    vaultPath: string,
    pluginId: string,
    operation: SettingsBridgeRequestOperation,
  ) => invoke<void>("launch_managed_settings_bridge_request", { vaultPath, pluginId, operation }),
  installLocalPlugin: (
    vaultPath: string,
    sourceFolderPath: string,
    overwriteExisting: boolean,
    obsidianClosedConfirmed: boolean,
  ) =>
    invoke<SyncSummary>("install_local_plugin", {
      vaultPath,
      sourceFolderPath,
      overwriteExisting,
      obsidianClosedConfirmed,
    }),
  deleteManagedPlugin: (
    vaultPath: string,
    pluginId: string,
    deleteConfirmed: boolean,
    secondaryConfirmed: boolean,
    obsidianClosedConfirmed: boolean,
  ) =>
    invoke<SyncSummary>("delete_managed_plugin", {
      vaultPath,
      pluginId,
      deleteConfirmed,
      secondaryConfirmed,
      obsidianClosedConfirmed,
    }),
  openManagedPluginFolder: (vaultPath: string, pluginId: string) =>
    invoke<void>("open_managed_plugin_folder", { vaultPath, pluginId }),
  buildVaultDiff: (sourceVaultPath: string, targetVaultPaths: string[]) =>
    invoke<TargetDiff[]>("build_vault_diff", { sourceVaultPath, targetVaultPaths }),
  checkObsidianRunning: () => invoke<boolean>("check_obsidian_running"),
  applySyncPlan: (plan: SyncPlan) => invoke<SyncSummary>("apply_sync_plan", { plan }),
  listBackups: (vaultPath: string) => invoke<BackupInfo[]>("list_backups", { vaultPath }),
  restoreBackup: (vaultPath: string, backupPath: string, obsidianClosedConfirmed: boolean) =>
    invoke<SyncSummary>("restore_backup", {
      vaultPath,
      backupPath,
      obsidianClosedConfirmed,
    }),
};
