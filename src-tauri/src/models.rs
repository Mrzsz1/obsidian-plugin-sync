use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum VaultSource {
    ObsidianConfig,
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Vault {
    pub id: String,
    pub name: String,
    pub path: String,
    pub config_dir: String,
    pub source: VaultSource,
    pub valid: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum UnsupportedReason {
    MissingManifest,
    MalformedManifest,
    MissingId,
    LinkDirectory,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInventoryItem {
    pub id: Option<String>,
    pub folder_name: String,
    pub folder_path: String,
    pub manifest_path: String,
    pub name: Option<String>,
    pub version: Option<String>,
    pub enabled: bool,
    pub has_data_json: bool,
    pub valid: bool,
    pub unsupported_reason: Option<UnsupportedReason>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultInventory {
    pub vault: Vault,
    pub plugins: Vec<PluginInventoryItem>,
    pub enabled_plugin_ids: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedPluginItem {
    pub plugin: PluginInventoryItem,
    pub configuration: Option<Value>,
    pub configuration_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultPluginManagementInventory {
    pub vault: Vault,
    pub plugins: Vec<ManagedPluginItem>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PluginSettingsSchemaSource {
    Declarative,
    Imperative,
    Mixed,
    DataJson,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PluginSettingsCompleteness {
    Complete,
    Partial,
    Fallback,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PluginSettingSource {
    Declarative,
    Imperative,
    DataJson,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PluginSettingConfidence {
    Exact,
    Inferred,
    Fallback,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PluginSettingSupport {
    SafeWritable,
    RiskTransform,
    DynamicExistingKey,
    ActionOnly,
    UnresolvedRuntime,
    UnsupportedCustom,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PluginSettingPortability {
    Portable,
    DeviceLocal,
    VaultLocal,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PluginSettingsAdapterStatus {
    Compatible,
    VersionMismatch,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PluginSettingControl {
    Toggle,
    Text,
    Textarea,
    Dropdown,
    Slider,
    Number,
    Color,
    Password,
    Heading,
    Nested,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PluginSettingOption {
    pub value: Value,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PluginSettingPathOption {
    pub path: String,
    pub label: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PluginSettingField {
    pub id: String,
    pub path: Option<String>,
    pub read_paths: Vec<String>,
    pub path_options: Vec<PluginSettingPathOption>,
    pub name: String,
    pub description: Option<String>,
    pub control: PluginSettingControl,
    pub options: Vec<PluginSettingOption>,
    pub placeholder: Option<String>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: Option<f64>,
    pub default_value: Option<Value>,
    pub source: PluginSettingSource,
    pub confidence: PluginSettingConfidence,
    pub support: PluginSettingSupport,
    pub read_only: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PluginSettingGroup {
    pub id: String,
    pub title: Option<String>,
    pub page_path: Vec<String>,
    pub fields: Vec<PluginSettingField>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PluginSettingsCoverage {
    pub total: usize,
    pub safe_writable: usize,
    pub risk_transform: usize,
    pub dynamic_existing_key: usize,
    pub action_only: usize,
    pub unresolved_runtime: usize,
    pub unsupported_custom: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PluginRuntimeSettingField {
    pub page_path: Vec<String>,
    pub group_title: Option<String>,
    pub order: usize,
    pub name: String,
    pub description: Option<String>,
    pub control: PluginSettingControl,
    pub options: Vec<PluginSettingOption>,
    pub placeholder: Option<String>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: Option<f64>,
    pub disabled: bool,
    pub visible: bool,
    pub action: bool,
    pub confidence: PluginSettingConfidence,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PluginRuntimeSettingsSnapshot {
    pub protocol_version: u32,
    pub plugin_id: String,
    pub plugin_version: Option<String>,
    pub fields: Vec<PluginRuntimeSettingField>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SettingsBridgeInstallationStatus {
    Missing,
    Disabled,
    Ready,
    VersionMismatch,
    Invalid,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SettingsBridgeSnapshotStatus {
    Missing,
    Fresh,
    Stale,
    Invalid,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SettingsBridgeStatus {
    pub plugin_id: String,
    pub bridge_id: String,
    pub bundled_version: String,
    pub installed_version: Option<String>,
    pub installation: SettingsBridgeInstallationStatus,
    pub enabled: bool,
    pub protocol_version: u32,
    pub snapshot: SettingsBridgeSnapshotStatus,
    pub captured_at: Option<String>,
    pub field_count: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SettingsBridgeRequestOperation {
    Capture,
    OpenSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PluginAdapterSettingField {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub control: PluginSettingControl,
    pub options: Vec<PluginSettingOption>,
    pub value: Value,
    pub default_value: Option<Value>,
    pub portability: PluginSettingPortability,
    pub writable: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PluginSettingsAdapterInfo {
    pub id: String,
    pub name: String,
    pub plugin_id: String,
    pub installed_version: Option<String>,
    pub version_requirement: String,
    pub status: PluginSettingsAdapterStatus,
    pub fields: Vec<PluginAdapterSettingField>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PluginAdapterSettingChange {
    pub field_id: String,
    pub value: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PluginSettingsSchema {
    pub source: PluginSettingsSchemaSource,
    pub completeness: PluginSettingsCompleteness,
    pub coverage: PluginSettingsCoverage,
    pub groups: Vec<PluginSettingGroup>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ManagedPluginSettings {
    pub plugin_id: String,
    pub configuration: Option<Value>,
    pub configuration_error: Option<String>,
    pub schema: PluginSettingsSchema,
    pub runtime_snapshot: Option<PluginRuntimeSettingsSnapshot>,
    pub bridge: SettingsBridgeStatus,
    pub adapter: Option<PluginSettingsAdapterInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RawPluginConfiguration {
    pub plugin_id: String,
    pub exists: bool,
    pub byte_length: usize,
    pub revision: String,
    pub raw_text: String,
    pub value: Option<Value>,
    pub parse_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RawConfigDiffOperation {
    Add,
    Change,
    Remove,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RawConfigDiffEntry {
    pub path: String,
    pub operation: RawConfigDiffOperation,
    pub before_exists: bool,
    pub before: Value,
    pub after_exists: bool,
    pub after: Value,
    pub sensitive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RawConfigDiffPreview {
    pub plugin_id: String,
    pub current_exists: bool,
    pub current_revision: String,
    pub current_parse_error: Option<String>,
    pub entries: Vec<RawConfigDiffEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PluginDiffStatus {
    MissingInTarget,
    SameVersion,
    SourceNewer,
    SourceOlder,
    VersionDifferentUnknown,
    TargetOnly,
    Invalid,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginDiffChecks {
    pub plugin_files_equal: bool,
    pub data_json_equal: bool,
    pub enabled_state_equal: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginDiff {
    pub plugin_id: String,
    pub display_name: String,
    pub status: PluginDiffStatus,
    pub checks: PluginDiffChecks,
    pub source_plugin: Option<PluginInventoryItem>,
    pub target_plugin: Option<PluginInventoryItem>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetDiff {
    pub target_vault: Vault,
    pub plugins: Vec<PluginDiff>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectedPluginOperation {
    pub plugin_id: String,
    pub source_vault_path: String,
    pub target_vault_path: String,
    pub copy_plugin_files: bool,
    pub sync_data_json: bool,
    pub sync_enabled_state: bool,
    pub delete_target_plugin: bool,
    pub force_downgrade: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPlan {
    pub source_vault_path: String,
    pub target_vault_paths: Vec<String>,
    pub operations: Vec<SelectedPluginOperation>,
    pub obsidian_closed_confirmed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationResult {
    pub plugin_id: Option<String>,
    pub target_vault_path: String,
    pub action: String,
    pub status: OperationStatus,
    pub message: String,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OperationStatus {
    Success,
    Skipped,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncSummary {
    pub started_at: String,
    pub finished_at: String,
    #[serde(default = "crate::models::current_app_version")]
    pub app_version: String,
    pub source_vault_path: Option<String>,
    pub target_vault_paths: Vec<String>,
    pub backup_paths: Vec<String>,
    pub results: Vec<OperationResult>,
}

pub fn current_app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupInfo {
    pub vault_path: String,
    pub backup_path: String,
    pub created_at: String,
    pub report_path: Option<String>,
    pub kind: Option<String>,
    pub plugin_id: Option<String>,
    pub operation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalPluginInstallPreview {
    pub plugin_id: String,
    pub name: String,
    pub incoming_version: Option<String>,
    pub existing_version: Option<String>,
    pub source_folder_path: String,
    pub will_overwrite: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub manual_vault_paths: Vec<String>,
    pub last_source_vault_path: Option<String>,
    pub last_target_vault_paths: Vec<String>,
}
