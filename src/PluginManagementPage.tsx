import { useEffect, useMemo, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import {
  AlertTriangle,
  Box,
  Check,
  FolderOpen,
  History,
  LoaderCircle,
  PackagePlus,
  Puzzle,
  RefreshCw,
  RotateCcw,
  Save,
  Search,
  Settings2,
  ShieldAlert,
  ShieldCheck,
  Trash2,
  X,
} from "lucide-react";
import { api } from "./api";
import { SETTINGS_BRIDGE_PLUGIN_ID } from "./bridge";
import { NativePluginSettingsEditor } from "./NativePluginSettingsEditor";
import { PluginAdapterSettingsEditor } from "./PluginAdapterSettingsEditor";
import { RawPluginConfigEditor } from "./RawPluginConfigEditor";
import { SettingsBridgePanel } from "./SettingsBridgePanel";
import type {
  BackupInfo,
  CommandError,
  JsonValue,
  LocalPluginInstallPreview,
  ManagedPluginItem,
  ManagedPluginSettings,
  SyncSummary,
  Vault,
  VaultPluginManagementInventory,
} from "./types";

type SortMode = "name" | "enabled" | "version";

type PluginManagementPageProps = {
  vault: Vault | null;
  backups: BackupInfo[];
  summary: SyncSummary | null;
  confirmObsidianClosed: () => Promise<boolean>;
  onBusyChange: (busy: boolean) => void;
  onRefreshShared: () => Promise<void>;
  onRestoreBackup: (backup: BackupInfo) => Promise<SyncSummary | null>;
  onSummary: (summary: SyncSummary) => void;
};

function commandMessage(error: unknown) {
  const commandError = error as Partial<CommandError>;
  if (commandError?.message) {
    return commandError.path ? `${commandError.message}：${commandError.path}` : commandError.message;
  }
  return error instanceof Error ? error.message : String(error);
}

function pluginKey(item: ManagedPluginItem) {
  return item.plugin.id ?? item.plugin.folderPath;
}

function displayName(item: ManagedPluginItem) {
  return item.plugin.name ?? item.plugin.folderName;
}

function cloneJson<T extends JsonValue>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}

function adapterValues(settings: ManagedPluginSettings | null) {
  return Object.fromEntries(
    (settings?.adapter?.fields ?? []).map((field) => [field.id, cloneJson(field.value)]),
  ) as Record<string, JsonValue>;
}

function versionText(value: string | null) {
  return value ? `v${value.replace(/^v/i, "")}` : "版本未知";
}

function backupOperationLabel(operation: string | null) {
  switch (operation) {
    case "enable":
      return "启用";
    case "disable":
      return "禁用";
    case "save-configuration":
      return "配置";
    case "save-configuration-risk-override":
      return "风险配置";
    case "save-adapter-configuration":
      return "适配配置";
    case "save-raw-configuration":
      return "原始配置";
    case "install":
      return "安装";
    case "overwrite-install":
      return "覆盖安装";
    case "delete":
      return "删除";
    case "restore":
      return "恢复前快照";
    default:
      return "插件操作";
  }
}

export function PluginManagementPage({
  vault,
  backups,
  summary,
  confirmObsidianClosed,
  onBusyChange,
  onRefreshShared,
  onRestoreBackup,
  onSummary,
}: PluginManagementPageProps) {
  const [inventory, setInventory] = useState<VaultPluginManagementInventory | null>(null);
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  const [sortMode, setSortMode] = useState<SortMode>("name");
  const [searchText, setSearchText] = useState("");
  const [loading, setLoading] = useState(false);
  const [mutationLabel, setMutationLabel] = useState<string | null>(null);
  const [messageText, setMessageText] = useState<string | null>(null);
  const [messageDanger, setMessageDanger] = useState(false);
  const [configOriginal, setConfigOriginal] = useState<JsonValue | null>(null);
  const [configDraft, setConfigDraft] = useState<JsonValue>({});
  const [configTouched, setConfigTouched] = useState(false);
  const [managedSettings, setManagedSettings] = useState<ManagedPluginSettings | null>(null);
  const [adapterOriginal, setAdapterOriginal] = useState<Record<string, JsonValue>>({});
  const [adapterDraft, setAdapterDraft] = useState<Record<string, JsonValue>>({});
  const [settingsLoading, setSettingsLoading] = useState(false);
  const [installPreview, setInstallPreview] = useState<LocalPluginInstallPreview | null>(null);
  const [overwriteConfirmed, setOverwriteConfirmed] = useState(false);
  const [deleteStep, setDeleteStep] = useState<0 | 1 | 2>(0);
  const [deleteFirstConfirmed, setDeleteFirstConfirmed] = useState(false);
  const [deleteSecondConfirmed, setDeleteSecondConfirmed] = useState(false);
  const [riskEditEnabled, setRiskEditEnabled] = useState(false);
  const [riskDialogOpen, setRiskDialogOpen] = useState(false);
  const [riskAcknowledged, setRiskAcknowledged] = useState(false);
  const [rawModeActive, setRawModeActive] = useState(false);
  const [settingsReloadToken, setSettingsReloadToken] = useState(0);
  const requestRef = useRef(0);
  const settingsRequestRef = useRef(0);
  const bridgeRefreshPendingRef = useRef(false);

  const manageablePlugins = useMemo(
    () => (inventory?.plugins ?? []).filter((item) => item.plugin.id !== SETTINGS_BRIDGE_PLUGIN_ID),
    [inventory],
  );

  const sortedPlugins = useMemo(() => {
    const query = searchText.trim().toLocaleLowerCase("zh-CN");
    return [...manageablePlugins]
      .filter((item) => {
        if (!query) return true;
        return [displayName(item), item.plugin.id ?? "", item.plugin.folderName]
          .join(" ")
          .toLocaleLowerCase("zh-CN")
          .includes(query);
      })
      .sort((left, right) => {
        if (sortMode === "enabled" && left.plugin.enabled !== right.plugin.enabled) {
          return left.plugin.enabled ? -1 : 1;
        }
        if (sortMode === "version") {
          const compared = (right.plugin.version ?? "").localeCompare(left.plugin.version ?? "", "zh-CN", {
            numeric: true,
          });
          if (compared !== 0) return compared;
        }
        return displayName(left).localeCompare(displayName(right), "zh-CN");
      });
  }, [manageablePlugins, searchText, sortMode]);

  const selectedPlugin =
    manageablePlugins.find((item) => pluginKey(item) === selectedKey) ?? sortedPlugins[0] ?? null;
  const selectedPluginId = selectedPlugin?.plugin.id ?? null;
  const configDirty = JSON.stringify(configDraft) !== JSON.stringify(configOriginal);
  const adapterDirty = JSON.stringify(adapterDraft) !== JSON.stringify(adapterOriginal);
  const standardDraftActive = configTouched && configDirty;
  const draftModeActive = standardDraftActive || adapterDirty || rawModeActive;
  const adapterChanges = useMemo(
    () => (managedSettings?.adapter?.fields ?? [])
      .filter((field) => field.writable)
      .filter(
        (field) => JSON.stringify(adapterDraft[field.id]) !== JSON.stringify(adapterOriginal[field.id]),
      )
      .map((field) => ({
        fieldId: field.id,
        value: Object.prototype.hasOwnProperty.call(adapterDraft, field.id)
          ? adapterDraft[field.id]
          : field.value,
      })),
    [adapterDraft, adapterOriginal, managedSettings?.adapter],
  );
  const riskEditableFieldCount = useMemo(
    () => managedSettings?.schema.groups
      .flatMap((group) => group.fields)
      .filter((field) => field.readOnly && (field.path !== null || field.pathOptions.length > 0)).length ?? 0,
    [managedSettings?.schema],
  );
  const managementBackups = backups.filter(
    (backup) =>
      backup.pluginId === selectedPluginId &&
      (backup.kind === "plugin-management" || backup.kind === "plugin-management-pre-restore"),
  );

  useEffect(() => {
    if (!vault) {
      setInventory(null);
      setSelectedKey(null);
      setManagedSettings(null);
      setConfigTouched(false);
      setAdapterOriginal({});
      setAdapterDraft({});
      return;
    }
    void loadInventory(vault.path);
  }, [vault?.path]);

  useEffect(() => {
    setRiskEditEnabled(false);
    setRiskDialogOpen(false);
    setRiskAcknowledged(false);
    setRawModeActive(false);
  }, [vault?.path, selectedKey, selectedPluginId]);

  useEffect(() => {
    const requestId = settingsRequestRef.current + 1;
    settingsRequestRef.current = requestId;
    if (!selectedPlugin) {
      setConfigOriginal(null);
      setConfigDraft({});
      setConfigTouched(false);
      setManagedSettings(null);
      setAdapterOriginal({});
      setAdapterDraft({});
      setSettingsLoading(false);
      return;
    }
    const hasReadableConfiguration = selectedPlugin.plugin.hasDataJson && !selectedPlugin.configurationError;
    const original = hasReadableConfiguration ? selectedPlugin.configuration : null;
    const draft = hasReadableConfiguration ? selectedPlugin.configuration : {};
    setConfigOriginal(original === null ? null : cloneJson(original));
    setConfigDraft(draft === null ? null : cloneJson(draft));
    setConfigTouched(false);
    setManagedSettings(null);
    setAdapterOriginal({});
    setAdapterDraft({});

    if (!vault || !selectedPlugin.plugin.valid || !selectedPlugin.plugin.id || selectedPlugin.plugin.unsupportedReason) {
      setSettingsLoading(false);
      return;
    }

    setSettingsLoading(true);
    void api.inspectManagedPluginSettings(vault.path, selectedPlugin.plugin.id)
      .then((next) => {
        if (settingsRequestRef.current !== requestId) return;
        setManagedSettings(next);
        const nextAdapterValues = adapterValues(next);
        setAdapterOriginal(nextAdapterValues);
        setAdapterDraft(cloneJson(nextAdapterValues));
        const readable = selectedPlugin.plugin.hasDataJson && !next.configurationError;
        const nextOriginal = readable ? next.configuration : null;
        const nextDraft = readable ? next.configuration : {};
        setConfigOriginal(nextOriginal === null ? null : cloneJson(nextOriginal));
        setConfigDraft(nextDraft === null ? null : cloneJson(nextDraft));
        setConfigTouched(false);
      })
      .catch((error) => {
        if (settingsRequestRef.current !== requestId) return;
        setMessageDanger(true);
        setMessageText(commandMessage(error));
      })
      .finally(() => {
        if (settingsRequestRef.current === requestId) setSettingsLoading(false);
      });
  }, [vault?.path, selectedKey, selectedPlugin?.configuration, selectedPlugin?.plugin.hasDataJson, settingsReloadToken]);

  useEffect(() => {
    function refreshAfterBridgeReturn() {
      if (!bridgeRefreshPendingRef.current) return;
      if (document.visibilityState === "hidden") return;
      bridgeRefreshPendingRef.current = false;
      setSettingsReloadToken((current) => current + 1);
      if (vault) void loadInventory(vault.path, false);
    }
    window.addEventListener("focus", refreshAfterBridgeReturn);
    document.addEventListener("visibilitychange", refreshAfterBridgeReturn);
    return () => {
      window.removeEventListener("focus", refreshAfterBridgeReturn);
      document.removeEventListener("visibilitychange", refreshAfterBridgeReturn);
    };
  }, [vault?.path]);

  async function loadInventory(path = vault?.path, clearMessage = true) {
    if (!path) return;
    const requestId = requestRef.current + 1;
    requestRef.current = requestId;
    setLoading(true);
    if (clearMessage) setMessageText(null);
    try {
      const next = await api.scanManagedPlugins(path);
      if (requestRef.current !== requestId) return;
      setInventory(next);
      setSelectedKey((current) => {
        const nextManageablePlugins = next.plugins.filter(
          (item) => item.plugin.id !== SETTINGS_BRIDGE_PLUGIN_ID,
        );
        if (current && nextManageablePlugins.some((item) => pluginKey(item) === current)) return current;
        return nextManageablePlugins[0] ? pluginKey(nextManageablePlugins[0]) : null;
      });
    } catch (error) {
      if (requestRef.current === requestId) {
        setMessageDanger(true);
        setMessageText(commandMessage(error));
      }
    } finally {
      if (requestRef.current === requestId) setLoading(false);
    }
  }

  async function runMutation(label: string, operation: () => Promise<SyncSummary>): Promise<SyncSummary | null> {
    if (!vault) return null;
    const confirmed = await confirmObsidianClosed();
    if (!confirmed) return null;
    setMutationLabel(label);
    setMessageText(null);
    onBusyChange(true);
    try {
      const nextSummary = await operation();
      onSummary(nextSummary);
      const failed = nextSummary.results.find((result) => result.status === "failed");
      setMessageDanger(Boolean(failed));
      setMessageText(failed ? failed.message : nextSummary.results[0]?.message ?? `${label}完成`);
      await Promise.all([loadInventory(vault.path, false), onRefreshShared()]);
      return nextSummary;
    } catch (error) {
      setMessageDanger(true);
      setMessageText(commandMessage(error));
      return null;
    } finally {
      setMutationLabel(null);
      onBusyChange(false);
    }
  }

  async function toggleEnabled() {
    if (!vault || !selectedPluginId || !selectedPlugin) return;
    const nextEnabled = !selectedPlugin.plugin.enabled;
    await runMutation(nextEnabled ? "启用插件" : "禁用插件", () =>
      api.setManagedPluginEnabled(vault.path, selectedPluginId, nextEnabled, true),
    );
  }

  async function saveConfiguration() {
    if (!vault || !selectedPluginId || !configDirty || adapterDirty || rawModeActive) return;
    await runMutation("保存配置", () =>
      api.saveManagedPluginConfiguration(vault.path, selectedPluginId, configDraft, true, riskEditEnabled),
    );
  }

  async function saveAdapterConfiguration() {
    const adapter = managedSettings?.adapter;
    if (
      !vault
      || !selectedPluginId
      || !adapter
      || adapterChanges.length === 0
      || standardDraftActive
      || rawModeActive
    ) return;
    await runMutation("保存适配设置", () =>
      api.saveManagedPluginAdapterConfiguration(
        vault.path,
        selectedPluginId,
        adapter.id,
        adapterChanges,
        true,
      ),
    );
  }

  async function saveRawConfiguration(proposed: JsonValue, expectedCurrentRevision: string) {
    if (!vault || !selectedPluginId) return null;
    return runMutation("保存原始配置", () =>
      api.saveRawManagedPluginConfiguration(
        vault.path,
        selectedPluginId,
        proposed,
        expectedCurrentRevision,
        true,
        true,
      ),
    );
  }

  async function refreshBridgeStatus() {
    if (vault) await loadInventory(vault.path, false);
    setSettingsReloadToken((current) => current + 1);
  }

  async function installBridge(allowDowngrade: boolean) {
    if (!vault) return;
    const result = await runMutation("安装 Bridge", () =>
      api.installManagedSettingsBridge(vault.path, true, allowDowngrade, true),
    );
    if (result) setSettingsReloadToken((current) => current + 1);
  }

  async function setBridgeEnabled(enabled: boolean) {
    if (!vault) return;
    const result = await runMutation(enabled ? "启用 Bridge" : "禁用 Bridge", () =>
      api.setManagedSettingsBridgeEnabled(vault.path, enabled, true),
    );
    if (result) setSettingsReloadToken((current) => current + 1);
  }

  async function removeBridge() {
    if (!vault) return;
    const result = await runMutation("移除 Bridge", () =>
      api.removeManagedSettingsBridge(vault.path, true, true),
    );
    if (result) setSettingsReloadToken((current) => current + 1);
  }

  async function launchBridgeRequest(operation: "capture" | "open-settings") {
    if (!vault || !selectedPluginId) return;
    bridgeRefreshPendingRef.current = true;
    try {
      await api.launchManagedSettingsBridgeRequest(vault.path, selectedPluginId, operation);
      setMessageDanger(false);
      setMessageText(
        operation === "capture"
          ? "已请求 Obsidian 抓取真实设置结构；返回本软件后会自动刷新"
          : "已在 Obsidian 中打开真实设置；返回本软件后会刷新配置",
      );
    } catch (error) {
      bridgeRefreshPendingRef.current = false;
      setMessageDanger(true);
      setMessageText(commandMessage(error));
    }
  }

  function updateStandardConfiguration(value: JsonValue) {
    if (adapterDirty) {
      setMessageDanger(false);
      setMessageText("请先保存或放弃适配设置更改");
      return;
    }
    setConfigDraft(value);
    setConfigTouched(true);
  }

  function updateAdapterConfiguration(fieldId: string, value: JsonValue) {
    if (standardDraftActive) {
      setMessageDanger(false);
      setMessageText("请先保存或放弃普通配置更改");
      return;
    }
    setAdapterDraft((current) => ({
      ...current,
      [fieldId]: cloneJson(value),
    }));
  }

  function resetStandardConfiguration() {
    setConfigDraft(configOriginal === null ? {} : cloneJson(configOriginal));
    setConfigTouched(false);
  }

  function resetAdapterConfiguration() {
    setAdapterDraft(cloneJson(adapterOriginal));
  }

  function closeRiskDialog() {
    setRiskDialogOpen(false);
    setRiskAcknowledged(false);
  }

  function confirmRiskEditing() {
    if (!riskAcknowledged) return;
    setRiskEditEnabled(true);
    closeRiskDialog();
  }

  async function chooseInstallFolder() {
    if (!vault) return;
    const selected = await open({
      directory: true,
      multiple: false,
      title: "选择本地 Obsidian 插件文件夹",
    });
    if (typeof selected !== "string") return;
    setMessageText(null);
    try {
      const preview = await api.inspectLocalPluginFolder(vault.path, selected);
      setInstallPreview(preview);
      setOverwriteConfirmed(false);
    } catch (error) {
      setMessageDanger(true);
      setMessageText(commandMessage(error));
    }
  }

  async function confirmInstall() {
    if (!vault || !installPreview) return;
    const preview = installPreview;
    setInstallPreview(null);
    await runMutation(preview.willOverwrite ? "覆盖安装插件" : "安装插件", () =>
      api.installLocalPlugin(vault.path, preview.sourceFolderPath, preview.willOverwrite, true),
    );
  }

  async function confirmDelete() {
    if (!vault || !selectedPluginId || !deleteFirstConfirmed || !deleteSecondConfirmed) return;
    setDeleteStep(0);
    setDeleteFirstConfirmed(false);
    setDeleteSecondConfirmed(false);
    await runMutation("删除插件", () =>
      api.deleteManagedPlugin(vault.path, selectedPluginId, true, true, true),
    );
  }

  async function openPluginFolder() {
    if (!vault || !selectedPluginId) return;
    setMessageText(null);
    try {
      await api.openManagedPluginFolder(vault.path, selectedPluginId);
    } catch (error) {
      setMessageDanger(true);
      setMessageText(commandMessage(error));
    }
  }

  async function restorePluginBackup(backup: BackupInfo) {
    onBusyChange(true);
    setMutationLabel("恢复插件");
    setMessageText(null);
    try {
      const restored = await onRestoreBackup(backup);
      if (restored) {
        setMessageDanger(restored.results.some((result) => result.status === "failed"));
        setMessageText(restored.results[0]?.message ?? "插件恢复完成");
      } else {
        setMessageDanger(true);
        setMessageText("恢复已取消或未能完成");
      }
      if (vault) await loadInventory(vault.path, false);
    } finally {
      setMutationLabel(null);
      onBusyChange(false);
    }
  }

  if (!vault) {
    return (
      <div className="manager-empty">
        <ShieldCheck size={30} />
        <strong>请选择一个知识库</strong>
        <span>插件管理会显示所选知识库中的本地插件。</span>
      </div>
    );
  }

  return (
    <section className="plugin-manager-page" aria-label="单库插件管理">
      <header className="manager-toolbar">
        <div>
          <p className="manager-eyebrow">当前知识库</p>
          <h2>{vault.name}</h2>
          <span>{manageablePlugins.length} 个插件</span>
        </div>
        <div className="manager-toolbar-actions">
          {mutationLabel && (
            <span className="manager-progress">
              <LoaderCircle className="spin-icon" size={16} />
              {mutationLabel}
            </span>
          )}
          <button
            className="ghost-action"
            onClick={() => void loadInventory()}
            disabled={loading || Boolean(mutationLabel) || draftModeActive}
            title={draftModeActive ? "请先保存、放弃或退出当前配置编辑" : "刷新插件列表"}
          >
            <RefreshCw className={loading ? "spin-icon" : ""} size={17} />
            刷新
          </button>
          <button
            className="primary-action"
            onClick={() => void chooseInstallFolder()}
            disabled={Boolean(mutationLabel) || draftModeActive}
            title={draftModeActive ? "请先保存、放弃或退出当前配置编辑" : "安装本地插件"}
          >
            <PackagePlus size={17} />
            安装本地插件
          </button>
        </div>
      </header>

      {messageText && (
        <div className={`manager-notice ${messageDanger ? "danger" : "info"}`}>
          {messageDanger ? <AlertTriangle size={16} /> : <Check size={16} />}
          <span>{messageText}</span>
        </div>
      )}

      <div className="manager-content">
        <aside className="manager-plugin-list">
          <div className="manager-list-tools">
            <label className="manager-search">
              <Search size={15} />
              <input value={searchText} onChange={(event) => setSearchText(event.target.value)} placeholder="搜索插件" />
            </label>
            <select value={sortMode} onChange={(event) => setSortMode(event.target.value as SortMode)} aria-label="插件排序">
              <option value="name">按名称</option>
              <option value="enabled">启用优先</option>
              <option value="version">按版本</option>
            </select>
          </div>

          <div className="manager-list-scroll">
            {sortedPlugins.map((item) => {
              const key = pluginKey(item);
              const unsupported = !item.plugin.valid || Boolean(item.plugin.unsupportedReason);
              const selectionBlocked = draftModeActive && selectedKey !== key;
              return (
                <button
                  key={key}
                  className={`manager-plugin-row ${selectedKey === key ? "active" : ""}`}
                  onClick={() => setSelectedKey(key)}
                  disabled={selectionBlocked}
                  title={selectionBlocked ? "请先保存、放弃或退出当前配置编辑" : item.plugin.folderPath}
                >
                  <span className="manager-plugin-icon">
                    <Box size={20} />
                  </span>
                  <span className="manager-plugin-copy">
                    <strong>{displayName(item)}</strong>
                    <small>{item.plugin.id ?? item.plugin.folderName}</small>
                  </span>
                  <span className={`manager-state-dot ${unsupported ? "warning" : item.plugin.enabled ? "success" : "neutral"}`} />
                </button>
              );
            })}
            {!loading && sortedPlugins.length === 0 && <p className="manager-list-empty">没有匹配的插件。</p>}
            {loading && (
              <div className="manager-loading">
                <LoaderCircle className="spin-icon" size={18} />
                正在扫描插件
              </div>
            )}
          </div>
        </aside>

        {selectedPlugin ? (
          <article className="manager-detail">
            <header className="manager-plugin-header">
              <span className="manager-plugin-icon large">
                <Box size={26} />
              </span>
              <div className="manager-plugin-identity">
                <h3>{displayName(selectedPlugin)}</h3>
                <p>{selectedPlugin.plugin.id ?? selectedPlugin.plugin.folderName}</p>
              </div>
              <span className="manager-version">{versionText(selectedPlugin.plugin.version)}</span>
              {selectedPlugin.plugin.valid && selectedPluginId && (
                <button
                  className={`manager-toggle ${selectedPlugin.plugin.enabled ? "enabled" : ""}`}
                  role="switch"
                  aria-checked={selectedPlugin.plugin.enabled}
                  onClick={() => void toggleEnabled()}
                  disabled={Boolean(mutationLabel) || draftModeActive}
                >
                  <span />
                  {selectedPlugin.plugin.enabled ? "已启用" : "已禁用"}
                </button>
              )}
            </header>

            {(!selectedPlugin.plugin.valid || selectedPlugin.plugin.unsupportedReason) && (
              <div className="manager-warning-panel">
                <AlertTriangle size={18} />
                <div>
                  <strong>
                    {selectedPlugin.plugin.unsupportedReason === "link-directory" ? "不支持链接目录" : "插件目录无效"}
                  </strong>
                  {selectedPlugin.plugin.warnings.map((warning) => <p key={warning}>{warning}</p>)}
                </div>
              </div>
            )}

            {selectedPlugin.plugin.valid && selectedPluginId && (
              <>
                <section className="manager-section config-section">
                  <div className="manager-section-heading">
                    <div>
                      <Settings2 size={17} />
                      <span>插件配置</span>
                    </div>
                    <div className="manager-section-actions">
                      {riskEditableFieldCount > 0 && (
                        <button
                          className={`ghost-action mini risk-edit-action ${riskEditEnabled ? "active" : ""}`}
                          onClick={() => {
                            if (riskEditEnabled) {
                              setRiskEditEnabled(false);
                            } else {
                              setRiskDialogOpen(true);
                            }
                          }}
                          disabled={Boolean(mutationLabel) || settingsLoading || adapterDirty || rawModeActive}
                        >
                          <ShieldAlert size={15} />
                          {riskEditEnabled ? "关闭风险编辑" : "允许风险编辑"}
                        </button>
                      )}
                      <button className="ghost-action mini" onClick={() => void openPluginFolder()}>
                        <FolderOpen size={15} />
                        打开目录
                      </button>
                      {standardDraftActive && (
                        <button
                          className="ghost-action mini"
                          onClick={resetStandardConfiguration}
                          disabled={Boolean(mutationLabel)}
                        >
                          <RotateCcw size={15} />
                          放弃更改
                        </button>
                      )}
                      <button
                        className="primary-action mini"
                        onClick={() => void saveConfiguration()}
                        disabled={
                          !configDirty
                          || adapterDirty
                          || Boolean(mutationLabel)
                          || settingsLoading
                          || rawModeActive
                        }
                      >
                        {mutationLabel === "保存配置" ? <LoaderCircle className="spin-icon" size={15} /> : <Save size={15} />}
                        保存配置
                      </button>
                    </div>
                  </div>
                  {(managedSettings?.configurationError ?? selectedPlugin.configurationError) && (
                    <div className="inline-warning">
                      <AlertTriangle size={15} />
                      {managedSettings?.configurationError ?? selectedPlugin.configurationError}
                    </div>
                  )}
                  {riskEditEnabled && (
                    <div className="risk-edit-warning" role="status">
                      <ShieldAlert size={16} />
                      <span>风险编辑仅对当前插件本次选择有效。保存时不会执行插件原有的转换、校验和保存逻辑。</span>
                    </div>
                  )}
                  <NativePluginSettingsEditor
                    schema={managedSettings?.schema ?? null}
                    value={configDraft}
                    loading={settingsLoading}
                    disabled={rawModeActive || adapterDirty || Boolean(mutationLabel)}
                    allowRiskyEdits={riskEditEnabled}
                    onChange={updateStandardConfiguration}
                  />
                </section>

                {managedSettings?.adapter && (
                  <section className="manager-section adapter-section">
                    <div className="manager-section-heading">
                      <div>
                        <Puzzle size={17} />
                        <span>内置适配器</span>
                      </div>
                      <div className="manager-section-actions">
                        {adapterDirty && (
                          <button
                            className="ghost-action mini"
                            onClick={resetAdapterConfiguration}
                            disabled={Boolean(mutationLabel)}
                          >
                            <RotateCcw size={15} />
                            放弃更改
                          </button>
                        )}
                        <button
                          className="primary-action mini"
                          onClick={() => void saveAdapterConfiguration()}
                          disabled={
                            !adapterDirty
                            || adapterChanges.length === 0
                            || standardDraftActive
                            || rawModeActive
                            || managedSettings.adapter.status !== "compatible"
                            || Boolean(mutationLabel)
                            || settingsLoading
                          }
                        >
                          {mutationLabel === "保存适配设置" ? (
                            <LoaderCircle className="spin-icon" size={15} />
                          ) : (
                            <Save size={15} />
                          )}
                          保存适配设置
                        </button>
                      </div>
                    </div>
                    <PluginAdapterSettingsEditor
                      adapter={managedSettings.adapter}
                      values={adapterDraft}
                      disabled={Boolean(mutationLabel) || settingsLoading || standardDraftActive || rawModeActive}
                      onChange={updateAdapterConfiguration}
                    />
                  </section>
                )}

                <SettingsBridgePanel
                  status={managedSettings?.bridge ?? null}
                  loading={settingsLoading}
                  disabled={Boolean(mutationLabel)}
                  draftBlocked={draftModeActive}
                  onRefresh={refreshBridgeStatus}
                  onInstall={installBridge}
                  onSetEnabled={setBridgeEnabled}
                  onRemove={removeBridge}
                  onCapture={() => launchBridgeRequest("capture")}
                  onOpenSettings={() => launchBridgeRequest("open-settings")}
                />

                <RawPluginConfigEditor
                  key={`${vault.path}:${selectedPluginId}`}
                  vaultPath={vault.path}
                  pluginId={selectedPluginId}
                  pluginName={displayName(selectedPlugin)}
                  disabled={Boolean(mutationLabel) || settingsLoading}
                  blockedByOtherDrafts={standardDraftActive || adapterDirty}
                  onModeChange={setRawModeActive}
                  onSave={saveRawConfiguration}
                />

                <section className="manager-section">
                  <div className="manager-section-heading">
                    <div>
                      <History size={17} />
                      <span>插件恢复记录</span>
                    </div>
                    <span className="section-count">{managementBackups.length}</span>
                  </div>
                  <div className="manager-history-list">
                    {managementBackups.slice(0, 8).map((backup) => (
                      <div className="manager-history-row" key={backup.backupPath}>
                        <span>
                          <strong>{backup.createdAt}</strong>
                          <small>{backupOperationLabel(backup.operation)}</small>
                        </span>
                        <button
                          className="ghost-action mini"
                          onClick={() => void restorePluginBackup(backup)}
                          disabled={Boolean(mutationLabel) || draftModeActive}
                        >
                          恢复
                        </button>
                      </div>
                    ))}
                    {managementBackups.length === 0 && <p className="manager-list-empty">暂无该插件的操作备份。</p>}
                  </div>
                </section>

                <section className="manager-danger-zone">
                  <div>
                    <strong>删除插件</strong>
                    <span>默认操作是禁用。删除会移除插件目录，并在写入前完整备份。</span>
                  </div>
                  <button
                    className="ghost-action danger"
                    onClick={() => setDeleteStep(1)}
                    disabled={Boolean(mutationLabel) || draftModeActive}
                  >
                    <Trash2 size={16} />
                    删除
                  </button>
                </section>
              </>
            )}
          </article>
        ) : (
          <div className="manager-empty compact">
            <Box size={28} />
            <strong>没有可显示的插件</strong>
          </div>
        )}
      </div>

      {installPreview && (
        <div className="close-overlay">
          <section className="manager-dialog" role="dialog" aria-modal="true" aria-labelledby="install-dialog-title">
            <header>
              <div>
                <p>本地插件安装</p>
                <h2 id="install-dialog-title">{installPreview.name}</h2>
              </div>
              <button className="icon-button" onClick={() => setInstallPreview(null)} title="关闭">
                <X size={17} />
              </button>
            </header>
            <div className="manager-dialog-body">
              <dl className="install-version-grid">
                <div><dt>插件 ID</dt><dd>{installPreview.pluginId}</dd></div>
                <div><dt>当前版本</dt><dd>{installPreview.existingVersion ? versionText(installPreview.existingVersion) : "未安装"}</dd></div>
                <div><dt>待安装版本</dt><dd>{versionText(installPreview.incomingVersion)}</dd></div>
              </dl>
              {installPreview.willOverwrite && (
                <label className="confirm-check danger-check">
                  <input type="checkbox" checked={overwriteConfirmed} onChange={(event) => setOverwriteConfirmed(event.target.checked)} />
                  我确认备份并覆盖现有插件，保留目标库原配置
                </label>
              )}
            </div>
            <footer>
              <button className="ghost-action" onClick={() => setInstallPreview(null)}>取消</button>
              <button
                className="primary-action"
                onClick={() => void confirmInstall()}
                disabled={installPreview.willOverwrite && !overwriteConfirmed}
              >
                <PackagePlus size={16} />
                {installPreview.willOverwrite ? "备份并覆盖" : "安装插件"}
              </button>
            </footer>
          </section>
        </div>
      )}

      {deleteStep > 0 && selectedPlugin && (
        <div className="close-overlay">
          <section className="manager-dialog danger-dialog" role="dialog" aria-modal="true" aria-labelledby="delete-dialog-title">
            <header>
              <div>
                <p>危险操作 · 第 {deleteStep} 次确认</p>
                <h2 id="delete-dialog-title">删除 {displayName(selectedPlugin)}</h2>
              </div>
              <button className="icon-button" onClick={() => setDeleteStep(0)} title="关闭"><X size={17} /></button>
            </header>
            <div className="manager-dialog-body">
              {deleteStep === 1 ? (
                <label className="confirm-check danger-check">
                  <input type="checkbox" checked={deleteFirstConfirmed} onChange={(event) => setDeleteFirstConfirmed(event.target.checked)} />
                  删除插件目录并从启用列表移除，操作前创建恢复备份
                </label>
              ) : (
                <label className="confirm-check danger-check">
                  <input type="checkbox" checked={deleteSecondConfirmed} onChange={(event) => setDeleteSecondConfirmed(event.target.checked)} />
                  我再次确认删除插件 {selectedPlugin.plugin.id}
                </label>
              )}
            </div>
            <footer>
              <button className="ghost-action" onClick={() => setDeleteStep(0)}>取消</button>
              {deleteStep === 1 ? (
                <button className="ghost-action danger" onClick={() => setDeleteStep(2)} disabled={!deleteFirstConfirmed}>继续确认</button>
              ) : (
                <button className="ghost-action danger" onClick={() => void confirmDelete()} disabled={!deleteSecondConfirmed}>
                  <Trash2 size={16} />
                  确认删除
                </button>
              )}
            </footer>
          </section>
        </div>
      )}

      {riskDialogOpen && selectedPlugin && (
        <div className="close-overlay">
          <section className="manager-dialog risk-dialog" role="dialog" aria-modal="true" aria-labelledby="risk-dialog-title">
            <header>
              <div>
                <p>临时风险授权</p>
                <h2 id="risk-dialog-title">允许编辑 {displayName(selectedPlugin)} 的受限设置</h2>
              </div>
              <button className="icon-button" onClick={closeRiskDialog} title="关闭"><X size={17} /></button>
            </header>
            <div className="manager-dialog-body">
              <div className="risk-dialog-explanation">
                <ShieldAlert size={20} />
                <span>将解锁 {riskEditableFieldCount} 项已识别路径的设置。切换插件、切换知识库或重启软件后会自动恢复只读。</span>
              </div>
              <label className="confirm-check risk-check">
                <input
                  type="checkbox"
                  checked={riskAcknowledged}
                  onChange={(event) => setRiskAcknowledged(event.target.checked)}
                />
                我理解本软件不会执行插件原有的转换、校验和保存逻辑，错误值可能导致插件异常。
              </label>
            </div>
            <footer>
              <button className="ghost-action" onClick={closeRiskDialog}>取消</button>
              <button className="ghost-action danger" onClick={confirmRiskEditing} disabled={!riskAcknowledged}>
                <ShieldAlert size={16} />
                允许风险编辑
              </button>
            </footer>
          </section>
        </div>
      )}
    </section>
  );
}
