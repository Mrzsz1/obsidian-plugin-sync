export type VaultSource = "obsidian-config" | "manual";

export type Vault = {
  id: string;
  name: string;
  path: string;
  configDir: string;
  source: VaultSource;
  valid: boolean;
  warnings: string[];
};

export type UnsupportedReason =
  | "missing-manifest"
  | "malformed-manifest"
  | "missing-id"
  | "link-directory";

export type PluginInventoryItem = {
  id: string | null;
  folderName: string;
  folderPath: string;
  manifestPath: string;
  name: string | null;
  version: string | null;
  enabled: boolean;
  hasDataJson: boolean;
  valid: boolean;
  unsupportedReason: UnsupportedReason | null;
  warnings: string[];
};

export type VaultInventory = {
  vault: Vault;
  plugins: PluginInventoryItem[];
  enabledPluginIds: string[];
  warnings: string[];
};

export type JsonValue = null | boolean | number | string | JsonValue[] | { [key: string]: JsonValue };

export type ManagedPluginItem = {
  plugin: PluginInventoryItem;
  configuration: JsonValue | null;
  configurationError: string | null;
};

export type VaultPluginManagementInventory = {
  vault: Vault;
  plugins: ManagedPluginItem[];
  warnings: string[];
};

export type PluginSettingsSchemaSource = "declarative" | "imperative" | "mixed" | "data-json";
export type PluginSettingsCompleteness = "complete" | "partial" | "fallback";
export type PluginSettingSource = "declarative" | "imperative" | "data-json";
export type PluginSettingConfidence = "exact" | "inferred" | "fallback";
export type PluginSettingSupport =
  | "safe-writable"
  | "risk-transform"
  | "dynamic-existing-key"
  | "action-only"
  | "unresolved-runtime"
  | "unsupported-custom";
export type PluginSettingPortability = "portable" | "device-local" | "vault-local";
export type PluginSettingsAdapterStatus = "compatible" | "version-mismatch";
export type PluginSettingControl =
  | "toggle"
  | "text"
  | "textarea"
  | "dropdown"
  | "slider"
  | "number"
  | "color"
  | "password"
  | "heading"
  | "nested"
  | "unsupported";

export type PluginSettingOption = {
  value: JsonValue;
  label: string;
};

export type PluginSettingPathOption = {
  path: string;
  label: string;
  detail: string;
};

export type PluginSettingField = {
  id: string;
  path: string | null;
  readPaths: string[];
  pathOptions: PluginSettingPathOption[];
  name: string;
  description: string | null;
  control: PluginSettingControl;
  options: PluginSettingOption[];
  placeholder: string | null;
  min: number | null;
  max: number | null;
  step: number | null;
  defaultValue: JsonValue | null;
  source: PluginSettingSource;
  confidence: PluginSettingConfidence;
  support: PluginSettingSupport;
  readOnly: boolean;
  warnings: string[];
};

export type PluginSettingGroup = {
  id: string;
  title: string | null;
  pagePath: string[];
  fields: PluginSettingField[];
};

export type PluginSettingsCoverage = {
  total: number;
  safeWritable: number;
  riskTransform: number;
  dynamicExistingKey: number;
  actionOnly: number;
  unresolvedRuntime: number;
  unsupportedCustom: number;
};

export type PluginRuntimeSettingField = {
  pagePath: string[];
  groupTitle: string | null;
  order: number;
  name: string;
  description: string | null;
  control: PluginSettingControl;
  options: PluginSettingOption[];
  placeholder: string | null;
  min: number | null;
  max: number | null;
  step: number | null;
  disabled: boolean;
  visible: boolean;
  action: boolean;
  confidence: PluginSettingConfidence;
};

export type PluginRuntimeSettingsSnapshot = {
  protocolVersion: number;
  pluginId: string;
  pluginVersion: string | null;
  fields: PluginRuntimeSettingField[];
  warnings: string[];
};

export type SettingsBridgeInstallationStatus =
  | "missing"
  | "disabled"
  | "ready"
  | "version-mismatch"
  | "invalid";

export type SettingsBridgeSnapshotStatus = "missing" | "fresh" | "stale" | "invalid";
export type SettingsBridgeRequestOperation = "capture" | "open-settings";

export type SettingsBridgeStatus = {
  pluginId: string;
  bridgeId: string;
  bundledVersion: string;
  installedVersion: string | null;
  installation: SettingsBridgeInstallationStatus;
  enabled: boolean;
  protocolVersion: number;
  snapshot: SettingsBridgeSnapshotStatus;
  capturedAt: string | null;
  fieldCount: number;
  warnings: string[];
};

export type PluginAdapterSettingField = {
  id: string;
  name: string;
  description: string | null;
  control: PluginSettingControl;
  options: PluginSettingOption[];
  value: JsonValue;
  defaultValue: JsonValue | null;
  portability: PluginSettingPortability;
  writable: boolean;
  warnings: string[];
};

export type PluginSettingsAdapterInfo = {
  id: string;
  name: string;
  pluginId: string;
  installedVersion: string | null;
  versionRequirement: string;
  status: PluginSettingsAdapterStatus;
  fields: PluginAdapterSettingField[];
  warnings: string[];
};

export type PluginAdapterSettingChange = {
  fieldId: string;
  value: JsonValue;
};

export type PluginSettingsSchema = {
  source: PluginSettingsSchemaSource;
  completeness: PluginSettingsCompleteness;
  coverage: PluginSettingsCoverage;
  groups: PluginSettingGroup[];
  warnings: string[];
};

export type ManagedPluginSettings = {
  pluginId: string;
  configuration: JsonValue | null;
  configurationError: string | null;
  schema: PluginSettingsSchema;
  runtimeSnapshot: PluginRuntimeSettingsSnapshot | null;
  bridge: SettingsBridgeStatus;
  adapter: PluginSettingsAdapterInfo | null;
};

export type RawPluginConfiguration = {
  pluginId: string;
  exists: boolean;
  byteLength: number;
  revision: string;
  rawText: string;
  value: JsonValue | null;
  parseError: string | null;
};

export type RawConfigDiffOperation = "add" | "change" | "remove";

export type RawConfigDiffEntry = {
  path: string;
  operation: RawConfigDiffOperation;
  beforeExists: boolean;
  before: JsonValue;
  afterExists: boolean;
  after: JsonValue;
  sensitive: boolean;
};

export type RawConfigDiffPreview = {
  pluginId: string;
  currentExists: boolean;
  currentRevision: string;
  currentParseError: string | null;
  entries: RawConfigDiffEntry[];
};

export type PluginDiffStatus =
  | "missing-in-target"
  | "same-version"
  | "source-newer"
  | "source-older"
  | "version-different-unknown"
  | "target-only"
  | "invalid"
  | "unsupported";

export type PluginDiff = {
  pluginId: string;
  displayName: string;
  status: PluginDiffStatus;
  checks: {
    pluginFilesEqual: boolean;
    dataJsonEqual: boolean;
    enabledStateEqual: boolean;
  };
  sourcePlugin: PluginInventoryItem | null;
  targetPlugin: PluginInventoryItem | null;
  warnings: string[];
};

export type TargetDiff = {
  targetVault: Vault;
  plugins: PluginDiff[];
  warnings: string[];
};

export type SelectedPluginOperation = {
  pluginId: string;
  sourceVaultPath: string;
  targetVaultPath: string;
  copyPluginFiles: boolean;
  syncDataJson: boolean;
  syncEnabledState: boolean;
  deleteTargetPlugin: boolean;
  forceDowngrade: boolean;
};

export type SyncPlan = {
  sourceVaultPath: string;
  targetVaultPaths: string[];
  operations: SelectedPluginOperation[];
  obsidianClosedConfirmed: boolean;
};

export type OperationResult = {
  pluginId: string | null;
  targetVaultPath: string;
  action: string;
  status: "success" | "skipped" | "failed";
  message: string;
  path: string | null;
};

export type SyncSummary = {
  startedAt: string;
  finishedAt: string;
  appVersion?: string;
  sourceVaultPath: string | null;
  targetVaultPaths: string[];
  backupPaths: string[];
  results: OperationResult[];
};

export type BackupInfo = {
  vaultPath: string;
  backupPath: string;
  createdAt: string;
  reportPath: string | null;
  kind: string | null;
  pluginId: string | null;
  operation: string | null;
};

export type LocalPluginInstallPreview = {
  pluginId: string;
  name: string;
  incomingVersion: string | null;
  existingVersion: string | null;
  sourceFolderPath: string;
  willOverwrite: boolean;
};

export type AppSettings = {
  manualVaultPaths: string[];
  lastSourceVaultPath: string | null;
  lastTargetVaultPaths: string[];
};

export type CommandError = {
  code: string;
  message: string;
  path?: string | null;
  details?: string | null;
};
