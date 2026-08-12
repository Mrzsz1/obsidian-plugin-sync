import { type CSSProperties, type PointerEvent as ReactPointerEvent, useEffect, useMemo, useRef, useState } from "react";
import { message, open } from "@tauri-apps/plugin-dialog";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  AlertTriangle,
  ArrowLeftRight,
  Box,
  Check,
  ChevronRight,
  DatabaseBackup,
  Folder,
  GripVertical,
  History,
  Info,
  LoaderCircle,
  Minimize2,
  PanelLeftClose,
  PanelLeftOpen,
  PanelRightClose,
  PanelRightOpen,
  Play,
  Plus,
  Power,
  RefreshCw,
  RotateCcw,
  Settings2,
  ShieldCheck,
  X,
} from "lucide-react";
import { api } from "./api";
import { SETTINGS_BRIDGE_PLUGIN_ID } from "./bridge";
import { PluginManagementPage } from "./PluginManagementPage";
import type {
  AppSettings,
  BackupInfo,
  CommandError,
  OperationResult,
  PluginDiff,
  SelectedPluginOperation,
  SyncSummary,
  TargetDiff,
  Vault,
  VaultInventory,
} from "./types";

type OperationMap = Record<string, SelectedPluginOperation>;
type ChipTone = "success" | "warning" | "danger" | "neutral" | "accent";
type SidebarSide = "left" | "right";
type WorkspacePage = "manage" | "sync";
type ObsidianWriteAction = "apply" | "restore" | "manage";

type DimensionSummary = {
  label: string;
  tone: ChipTone;
};

type PluginTargetState = {
  targetVault: Vault;
  diff: PluginDiff;
  operation: SelectedPluginOperation | undefined;
};

type PluginCardModel = {
  pluginId: string;
  displayName: string;
  sourcePlugin: PluginDiff["sourcePlugin"];
  targets: PluginTargetState[];
  selected: boolean;
  focused: boolean;
  versionText: string;
  status: DimensionSummary;
  files: DimensionSummary;
  settings: DimensionSummary;
  enabled: DimensionSummary;
  riskCount: number;
  operationCount: number;
  warnings: string[];
};

const emptySettings: AppSettings = {
  manualVaultPaths: [],
  lastSourceVaultPath: null,
  lastTargetVaultPaths: [],
};

const leftRailBounds = {
  min: 220,
  max: 360,
  default: 250,
};

const rightDrawerBounds = {
  min: 320,
  max: 520,
  default: 376,
};

function clamp(value: number, min: number, max: number) {
  return Math.min(max, Math.max(min, value));
}

function operationKey(targetVaultPath: string, pluginId: string) {
  return `${targetVaultPath}::${pluginId}`;
}

function commandMessage(error: unknown) {
  const commandError = error as Partial<CommandError>;
  if (commandError?.message) {
    return commandError.path ? `${commandError.message}：${commandError.path}` : commandError.message;
  }
  return error instanceof Error ? error.message : String(error);
}

function resultTone(status: OperationResult["status"]): ChipTone {
  if (status === "success") return "success";
  if (status === "failed") return "danger";
  return "neutral";
}

function versionLabel(version: string | null | undefined) {
  return version?.trim() ? version : "未知";
}

function diffVersionText(diff: PluginDiff) {
  const sourceVersion = diff.sourcePlugin ? versionLabel(diff.sourcePlugin.version) : "无";
  const targetVersion = diff.targetPlugin ? versionLabel(diff.targetPlugin.version) : "无";
  return `源 ${sourceVersion} → 目标 ${targetVersion}`;
}

function hashString(value: string) {
  let hash = 0;
  for (const char of value) {
    hash = (hash * 31 + char.charCodeAt(0)) >>> 0;
  }
  return hash;
}

function toneClass(value: string) {
  return `tone-${hashString(value) % 6}`;
}

function firstGlyph(value: string) {
  return Array.from(value.trim())[0] ?? "?";
}

function vaultInitial(vault: Vault) {
  return firstGlyph(vault.name || vault.path);
}

function isBlockedDiff(diff: PluginDiff) {
  return diff.status === "invalid" || diff.status === "unsupported";
}

function canOperateOnSourceBackedPlugin(diff: PluginDiff) {
  return !isBlockedDiff(diff) && diff.status !== "target-only";
}

function buildDefaultOperation(
  sourceVaultPath: string,
  targetVaultPath: string,
  diff: PluginDiff,
): SelectedPluginOperation | null {
  if (diff.pluginId === SETTINGS_BRIDGE_PLUGIN_ID || !canOperateOnSourceBackedPlugin(diff)) {
    return null;
  }

  const copyPluginFiles = diff.status !== "source-older" && !diff.checks.pluginFilesEqual;
  const syncDataJson = Boolean(diff.sourcePlugin?.hasDataJson) && !diff.checks.dataJsonEqual;
  const syncEnabledState = !diff.checks.enabledStateEqual;
  const next: SelectedPluginOperation = {
    pluginId: diff.pluginId,
    sourceVaultPath,
    targetVaultPath,
    copyPluginFiles: syncEnabledState && diff.status === "missing-in-target" ? true : copyPluginFiles,
    syncDataJson,
    syncEnabledState,
    deleteTargetPlugin: false,
    forceDowngrade: false,
  };

  if (!next.copyPluginFiles && !next.syncDataJson && !next.syncEnabledState) {
    return null;
  }

  return next;
}

function statusSummary(targets: PluginTargetState[]): DimensionSummary {
  if (targets.length === 0) return { label: "未比较", tone: "neutral" };
  if (targets.some(({ diff }) => diff.status === "invalid" || diff.status === "unsupported")) {
    return { label: "不可用", tone: "danger" };
  }
  if (targets.some(({ diff }) => diff.status === "source-older")) {
    return { label: "会降级", tone: "danger" };
  }
  if (targets.some(({ diff }) => diff.status === "missing-in-target")) {
    return { label: "目标缺失", tone: "warning" };
  }
  if (targets.some(({ diff }) => diff.status === "target-only")) {
    return { label: "目标独有", tone: "warning" };
  }
  if (targets.some(({ diff }) => diff.status === "source-newer" || diff.status === "version-different-unknown")) {
    return { label: "可同步", tone: "warning" };
  }
  if (
    targets.some(
      ({ diff }) =>
        !diff.checks.pluginFilesEqual || !diff.checks.dataJsonEqual || !diff.checks.enabledStateEqual,
    )
  ) {
    return { label: "状态不同", tone: "warning" };
  }
  return { label: "一致", tone: "success" };
}

function filesSummary(targets: PluginTargetState[]): DimensionSummary {
  if (targets.length === 0) return { label: "未比较", tone: "neutral" };
  if (targets.some(({ diff }) => diff.status === "invalid" || diff.status === "unsupported")) {
    return { label: "不可用", tone: "danger" };
  }
  if (targets.some(({ diff }) => diff.status === "missing-in-target")) {
    return { label: "缺失", tone: "danger" };
  }
  if (targets.some(({ diff }) => diff.status === "target-only")) {
    return { label: "目标独有", tone: "warning" };
  }
  if (targets.every(({ diff }) => diff.checks.pluginFilesEqual)) {
    return { label: "一致", tone: "success" };
  }
  return { label: "有差异", tone: "warning" };
}

function settingsSummary(targets: PluginTargetState[]): DimensionSummary {
  if (targets.length === 0) return { label: "未比较", tone: "neutral" };
  if (targets.some(({ diff }) => diff.status === "invalid" || diff.status === "unsupported")) {
    return { label: "不可用", tone: "danger" };
  }
  const anySettings = targets.some(({ diff }) => diff.sourcePlugin?.hasDataJson || diff.targetPlugin?.hasDataJson);
  if (!anySettings) {
    return { label: "未发现", tone: "neutral" };
  }
  if (targets.every(({ diff }) => diff.checks.dataJsonEqual)) {
    return { label: "一致", tone: "success" };
  }
  return { label: "有差异", tone: "warning" };
}

function enabledSummary(targets: PluginTargetState[]): DimensionSummary {
  if (targets.length === 0) return { label: "未比较", tone: "neutral" };
  if (targets.some(({ diff }) => diff.status === "invalid" || diff.status === "unsupported")) {
    return { label: "不可用", tone: "danger" };
  }
  if (!targets.every(({ diff }) => diff.checks.enabledStateEqual)) {
    return { label: "有差异", tone: "warning" };
  }
  const plugin = targets.find(({ diff }) => diff.sourcePlugin || diff.targetPlugin)?.diff.sourcePlugin ??
    targets.find(({ diff }) => diff.targetPlugin)?.diff.targetPlugin;
  if (!plugin) {
    return { label: "未发现", tone: "neutral" };
  }
  return plugin.enabled ? { label: "已启用", tone: "success" } : { label: "已禁用", tone: "neutral" };
}

function versionSummary(targets: PluginTargetState[]) {
  const versions = Array.from(
    new Set(
      targets.map(({ diff }) => diff.targetPlugin?.version?.trim()).filter((version): version is string => Boolean(version)),
    ),
  );
  if (versions.length === 0) {
    return "无目标版本";
  }
  if (versions.length === 1) {
    return `v${versions[0]}`;
  }
  return `${versions.length} 个版本`;
}

function operationLabels(operation: SelectedPluginOperation) {
  const labels: string[] = [];
  if (operation.copyPluginFiles) labels.push(operation.forceDowngrade ? "插件文件（降级）" : "插件文件");
  if (operation.syncDataJson) labels.push("设置");
  if (operation.syncEnabledState) labels.push("启用状态");
  if (operation.deleteTargetPlugin) labels.push("删除插件");
  return labels;
}

function StatusChip({ label, tone }: DimensionSummary) {
  return <span className={`status-chip ${tone}`}>{label}</span>;
}

function App() {
  const [settings, setSettings] = useState<AppSettings>(emptySettings);
  const [vaults, setVaults] = useState<Vault[]>([]);
  const [sourceVaultPath, setSourceVaultPath] = useState<string | null>(null);
  const [targetVaultPaths, setTargetVaultPaths] = useState<string[]>([]);
  const [inventory, setInventory] = useState<VaultInventory | null>(null);
  const [diffs, setDiffs] = useState<TargetDiff[]>([]);
  const [operations, setOperations] = useState<OperationMap>({});
  const [selectedPluginIds, setSelectedPluginIds] = useState<string[]>([]);
  const [focusedPluginId, setFocusedPluginId] = useState<string | null>(null);
  const [summary, setSummary] = useState<SyncSummary | null>(null);
  const [backups, setBackups] = useState<BackupInfo[]>([]);
  const [busy, setBusy] = useState(false);
  const [diffBusy, setDiffBusy] = useState(false);
  const [syncing, setSyncing] = useState(false);
  const [messageText, setMessageText] = useState<string | null>(null);
  const [workspacePage, setWorkspacePage] = useState<WorkspacePage>("manage");
  const [managerBusy, setManagerBusy] = useState(false);
  const [closeDialogOpen, setCloseDialogOpen] = useState(false);
  const [obsidianDialogAction, setObsidianDialogAction] = useState<ObsidianWriteAction | null>(null);
  const [leftRailOpen, setLeftRailOpen] = useState(true);
  const [rightDrawerOpen, setRightDrawerOpen] = useState(true);
  const [leftRailWidth, setLeftRailWidth] = useState(leftRailBounds.default);
  const [rightDrawerWidth, setRightDrawerWidth] = useState(rightDrawerBounds.default);
  const allowWindowCloseRef = useRef(false);
  const diffRequestRef = useRef(0);
  const obsidianConfirmResolverRef = useRef<((confirmed: boolean) => void) | null>(null);

  const sourceVault = vaults.find((vault) => vault.path === sourceVaultPath) ?? null;
  const selectedTargetVaults = vaults.filter((vault) => targetVaultPaths.includes(vault.path));
  const operationList = useMemo(() => Object.values(operations), [operations]);
  const deleteOperations = operationList.filter((operation) => operation.deleteTargetPlugin);
  const downgradeOperations = operationList.filter((operation) => operation.forceDowngrade);
  const riskCount = deleteOperations.length + downgradeOperations.length;
  const targetVaultKey = targetVaultPaths.join("\u0000");
  const selectedPluginSet = useMemo(() => new Set(selectedPluginIds), [selectedPluginIds]);

  const pluginCards = useMemo<PluginCardModel[]>(() => {
    const grouped = new Map<string, PluginTargetState[]>();

    for (const targetDiff of diffs) {
      for (const diff of targetDiff.plugins) {
        if (diff.pluginId === SETTINGS_BRIDGE_PLUGIN_ID) continue;
        const key = diff.pluginId;
        const states = grouped.get(key) ?? [];
        states.push({
          targetVault: targetDiff.targetVault,
          diff,
          operation: operations[operationKey(targetDiff.targetVault.path, diff.pluginId)],
        });
        grouped.set(key, states);
      }
    }

    return Array.from(grouped.entries())
      .map(([pluginId, targets]) => {
        const firstDiff = targets[0]?.diff;
        const sourcePlugin = targets.find(({ diff }) => diff.sourcePlugin)?.diff.sourcePlugin ?? null;
        const operationCount = targets.filter(({ operation }) => operation).length;
        return {
          pluginId,
          displayName: firstDiff?.displayName ?? pluginId,
          sourcePlugin,
          targets,
          selected: selectedPluginSet.has(pluginId),
          focused: focusedPluginId === pluginId,
          versionText: versionSummary(targets),
          status: statusSummary(targets),
          files: filesSummary(targets),
          settings: settingsSummary(targets),
          enabled: enabledSummary(targets),
          riskCount: targets.filter(
            ({ operation }) => operation?.deleteTargetPlugin || operation?.forceDowngrade,
          ).length,
          operationCount,
          warnings: targets.flatMap(({ diff }) => diff.warnings),
        } satisfies PluginCardModel;
      })
      .sort((a, b) => a.displayName.localeCompare(b.displayName, "zh-CN"));
  }, [diffs, focusedPluginId, operations, selectedPluginSet]);

  const focusedCard = pluginCards.find((card) => card.pluginId === focusedPluginId) ?? pluginCards[0] ?? null;
  const layoutStyle = {
    "--left-rail-track": leftRailOpen ? `${leftRailWidth}px` : "0px",
    "--left-chrome-track": leftRailOpen ? `${leftRailWidth}px` : "64px",
    "--right-drawer-track": workspacePage === "sync" && rightDrawerOpen ? `${rightDrawerWidth}px` : "0px",
  } as CSSProperties;

  useEffect(() => {
    void initialize();
  }, []);

  useEffect(() => {
    const appWindow = getCurrentWindow();
    let disposed = false;
    let unlisten: (() => void) | null = null;

    appWindow
      .onCloseRequested((event) => {
        if (allowWindowCloseRef.current) {
          return;
        }

        event.preventDefault();
        setCloseDialogOpen(true);
      })
      .then((cleanup) => {
        if (disposed) {
          cleanup();
        } else {
          unlisten = cleanup;
        }
      })
      .catch((error) => {
        setMessageText(commandMessage(error));
      });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (!sourceVaultPath) {
      setInventory(null);
      setBackups([]);
      return;
    }
    void loadInventory(sourceVaultPath);
  }, [sourceVaultPath]);

  useEffect(() => {
    clearOperationSelection();
    if (!sourceVaultPath || targetVaultPaths.length === 0) {
      setDiffs([]);
      return;
    }
    void refreshDiff({ clearSummary: true, showEmptyMessage: false });
  }, [sourceVaultPath, targetVaultKey]);

  useEffect(() => {
    if (pluginCards.length === 0) {
      setFocusedPluginId(null);
      return;
    }
    if (!focusedPluginId || !pluginCards.some((card) => card.pluginId === focusedPluginId)) {
      setFocusedPluginId(pluginCards[0].pluginId);
    }
  }, [focusedPluginId, pluginCards]);

  async function initialize() {
    setBusy(true);
    setMessageText(null);
    try {
      const loadedSettings = await api.loadAppSettings().catch(() => emptySettings);
      const discoveredVaults = await api.discoverVaults();
      const manualVaults = await Promise.all(
        loadedSettings.manualVaultPaths.map((path) => api.validateVaultPath(path).catch(() => null)),
      );
      const merged = mergeVaults([...discoveredVaults, ...manualVaults.filter((vault): vault is Vault => Boolean(vault))]);
      const initialSource = loadedSettings.lastSourceVaultPath ?? merged[0]?.path ?? null;
      const initialTargets = loadedSettings.lastTargetVaultPaths.filter((path) => path !== initialSource);

      setSettings(loadedSettings);
      setVaults(merged);
      setSourceVaultPath(initialSource);
      setTargetVaultPaths(initialTargets);
    } catch (error) {
      setMessageText(commandMessage(error));
    } finally {
      setBusy(false);
    }
  }

  function mergeVaults(items: Vault[]) {
    return Array.from(new Map(items.map((vault) => [vault.path.toLowerCase(), vault])).values()).sort((a, b) =>
      a.name.localeCompare(b.name, "zh-CN"),
    );
  }

  function clearOperationSelection() {
    setOperations({});
    setSelectedPluginIds([]);
  }

  async function persistSettings(next: AppSettings) {
    setSettings(next);
    await api.saveAppSettings(next);
  }

  async function loadInventory(path: string) {
    try {
      const nextInventory = await api.scanVault(path);
      setInventory(nextInventory);
      const nextBackups = await api.listBackups(path).catch(() => []);
      setBackups(nextBackups);
    } catch (error) {
      setMessageText(commandMessage(error));
    }
  }

  async function refreshVaults() {
    await initialize();
  }

  function startSidebarResize(side: SidebarSide, event: ReactPointerEvent<HTMLDivElement>) {
    event.preventDefault();

    const startX = event.clientX;
    const startWidth = side === "left" ? leftRailWidth : rightDrawerWidth;

    function resize(moveEvent: PointerEvent) {
      const delta = moveEvent.clientX - startX;
      if (side === "left") {
        setLeftRailWidth(clamp(startWidth + delta, leftRailBounds.min, leftRailBounds.max));
      } else {
        setRightDrawerWidth(clamp(startWidth - delta, rightDrawerBounds.min, rightDrawerBounds.max));
      }
    }

    function stopResize() {
      document.body.classList.remove("resizing-sidebar");
      window.removeEventListener("pointermove", resize);
      window.removeEventListener("pointerup", stopResize);
      window.removeEventListener("pointercancel", stopResize);
    }

    document.body.classList.add("resizing-sidebar");
    window.addEventListener("pointermove", resize);
    window.addEventListener("pointerup", stopResize);
    window.addEventListener("pointercancel", stopResize);
  }

  async function hideWindowToTray() {
    setCloseDialogOpen(false);
    try {
      await getCurrentWindow().hide();
    } catch (error) {
      setMessageText(commandMessage(error));
    }
  }

  async function closeWindowDirectly() {
    setCloseDialogOpen(false);
    allowWindowCloseRef.current = true;
    try {
      await getCurrentWindow().destroy();
    } catch (error) {
      allowWindowCloseRef.current = false;
      setMessageText(commandMessage(error));
    }
  }

  async function addVault() {
    const selected = await open({
      directory: true,
      multiple: false,
      title: "选择 Obsidian 知识库",
    });
    if (typeof selected !== "string") return;
    setBusy(true);
    setMessageText(null);
    try {
      const vault = await api.validateVaultPath(selected);
      const nextVaults = mergeVaults([...vaults, vault]);
      const nextSettings = {
        ...settings,
        manualVaultPaths: Array.from(new Set([...settings.manualVaultPaths, vault.path])),
        lastSourceVaultPath: vault.path,
      };
      setVaults(nextVaults);
      setSourceVaultPath(vault.path);
      setWorkspacePage("manage");
      await persistSettings(nextSettings);
    } catch (error) {
      setMessageText(commandMessage(error));
    } finally {
      setBusy(false);
    }
  }

  async function chooseSource(path: string) {
    const nextTargets = targetVaultPaths.filter((targetPath) => targetPath !== path);
    setSourceVaultPath(path);
    setWorkspacePage("manage");
    setTargetVaultPaths(nextTargets);
    setDiffs([]);
    setFocusedPluginId(null);
    clearOperationSelection();
    await persistSettings({
      ...settings,
      lastSourceVaultPath: path,
      lastTargetVaultPaths: nextTargets,
    });
  }

  async function toggleTarget(path: string) {
    if (path === sourceVaultPath) return;
    const nextTargets = targetVaultPaths.includes(path)
      ? targetVaultPaths.filter((targetPath) => targetPath !== path)
      : [...targetVaultPaths, path];
    setTargetVaultPaths(nextTargets);
    setDiffs([]);
    setFocusedPluginId(null);
    clearOperationSelection();
    await persistSettings({
      ...settings,
      lastSourceVaultPath: sourceVaultPath,
      lastTargetVaultPaths: nextTargets,
    });
  }

  async function refreshDiff(options?: { clearSummary?: boolean; showEmptyMessage?: boolean }) {
    const clearSummary = options?.clearSummary ?? false;
    const showEmptyMessage = options?.showEmptyMessage ?? true;
    if (!sourceVaultPath || targetVaultPaths.length === 0) {
      setDiffs([]);
      if (showEmptyMessage) {
        setMessageText("请选择一个源库和至少一个目标库。");
      }
      return;
    }

    const requestId = diffRequestRef.current + 1;
    diffRequestRef.current = requestId;
    setDiffBusy(true);
    setMessageText(null);
    if (clearSummary) {
      setSummary(null);
    }

    try {
      const nextDiffs = await api.buildVaultDiff(sourceVaultPath, targetVaultPaths);
      if (diffRequestRef.current === requestId) {
        setDiffs(nextDiffs);
      }
    } catch (error) {
      if (diffRequestRef.current === requestId) {
        setMessageText(commandMessage(error));
      }
    } finally {
      if (diffRequestRef.current === requestId) {
        setDiffBusy(false);
      }
    }
  }

  function togglePluginSelection(card: PluginCardModel) {
    if (!sourceVaultPath) return;
    setFocusedPluginId(card.pluginId);

    if (selectedPluginSet.has(card.pluginId)) {
      setSelectedPluginIds((current) => current.filter((pluginId) => pluginId !== card.pluginId));
      setOperations((current) =>
        Object.fromEntries(Object.entries(current).filter(([, operation]) => operation.pluginId !== card.pluginId)),
      );
      return;
    }

    setSelectedPluginIds((current) => (current.includes(card.pluginId) ? current : [...current, card.pluginId]));
    setOperations((current) => {
      const next = { ...current };
      for (const { targetVault, diff } of card.targets) {
        const defaultOperation = buildDefaultOperation(sourceVaultPath, targetVault.path, diff);
        if (defaultOperation) {
          next[operationKey(targetVault.path, diff.pluginId)] = defaultOperation;
        }
      }
      return next;
    });
  }

  function updateOperation(targetPath: string, diff: PluginDiff, patch: Partial<SelectedPluginOperation>) {
    if (!sourceVaultPath) return;
    const key = operationKey(targetPath, diff.pluginId);
    setFocusedPluginId(diff.pluginId);
    setSelectedPluginIds((selected) => (selected.includes(diff.pluginId) ? selected : [...selected, diff.pluginId]));
    setOperations((current) => {
      const previous =
        current[key] ??
        ({
          pluginId: diff.pluginId,
          sourceVaultPath,
          targetVaultPath: targetPath,
          copyPluginFiles: false,
          syncDataJson: false,
          syncEnabledState: false,
          deleteTargetPlugin: false,
          forceDowngrade: false,
        } satisfies SelectedPluginOperation);
      const next = { ...previous, ...patch };

      if (next.deleteTargetPlugin) {
        next.copyPluginFiles = false;
        next.syncDataJson = false;
        next.syncEnabledState = false;
        next.forceDowngrade = false;
      }
      if (next.syncEnabledState && diff.status === "missing-in-target") {
        next.copyPluginFiles = true;
      }
      if (diff.status !== "source-older") {
        next.forceDowngrade = false;
      }
      if (diff.status !== "target-only") {
        next.deleteTargetPlugin = false;
      }

      const hasAction = next.copyPluginFiles || next.syncDataJson || next.syncEnabledState || next.deleteTargetPlugin;
      if (!hasAction) {
        const { [key]: _removed, ...rest } = current;
        return rest;
      }

      return { ...current, [key]: next };
    });
  }

  function findDiff(targetVaultPath: string, pluginId: string) {
    return diffs
      .find((targetDiff) => targetDiff.targetVault.path === targetVaultPath)
      ?.plugins.find((pluginDiff) => pluginDiff.pluginId === pluginId);
  }

  async function confirmDangerousOperations() {
    const deletes = deleteOperations.map((operation) => {
      const diff = findDiff(operation.targetVaultPath, operation.pluginId);
      return `${diff?.displayName ?? operation.pluginId} -> ${operation.targetVaultPath}`;
    });
    const downgrades = downgradeOperations.map((operation) => {
      const diff = findDiff(operation.targetVaultPath, operation.pluginId);
      const versionText = diff ? diffVersionText(diff) : "源 未知 -> 目标 未知";
      return `${diff?.displayName ?? operation.pluginId} ${versionText} (${operation.targetVaultPath})`;
    });
    if (deletes.length === 0 && downgrades.length === 0) {
      return true;
    }

    const lines = ["即将执行危险操作，应用前会先备份："];
    if (deletes.length > 0) {
      lines.push("", "删除插件：", ...deletes.map((item) => `- ${item}`));
    }
    if (downgrades.length > 0) {
      lines.push("", "版本降级：", ...downgrades.map((item) => `- ${item}`));
    }

    const result = await message(lines.join("\n"), {
      title: "确认危险操作",
      kind: "warning",
      buttons: {
        ok: "继续同步",
        cancel: "取消",
      },
    });
    return result === "继续同步" || result === "Ok";
  }

  function confirmObsidianClosed(action: ObsidianWriteAction) {
    return new Promise<boolean>((resolve) => {
      obsidianConfirmResolverRef.current = resolve;
      setObsidianDialogAction(action);
    });
  }

  function finishObsidianConfirm(confirmed: boolean) {
    setObsidianDialogAction(null);
    const resolve = obsidianConfirmResolverRef.current;
    obsidianConfirmResolverRef.current = null;
    resolve?.(confirmed);
  }

  async function previewPlan() {
    if (operationList.length === 0) {
      setMessageText("没有选择任何同步操作。");
      return;
    }

    const lines = [
      `源库：${sourceVault?.name ?? sourceVaultPath ?? "未知"}`,
      `目标库：${selectedTargetVaults.length} 个`,
      `插件：${new Set(operationList.map((operation) => operation.pluginId)).size} 个`,
      `操作：${operationList.length} 项`,
      "",
    ];

    for (const operation of operationList.slice(0, 18)) {
      const target = vaults.find((vault) => vault.path === operation.targetVaultPath);
      const diff = findDiff(operation.targetVaultPath, operation.pluginId);
      lines.push(
        `- ${diff?.displayName ?? operation.pluginId} -> ${target?.name ?? operation.targetVaultPath}：${operationLabels(
          operation,
        ).join("、")}`,
      );
    }

    if (operationList.length > 18) {
      lines.push(`... 还有 ${operationList.length - 18} 项`);
    }

    lines.push("", "写入前会创建备份，删除和降级会再次确认。");
    await message(lines.join("\n"), {
      title: "预览变更",
      kind: "info",
      buttons: {
        ok: "知道了",
      },
    });
  }

  async function showSyncRecords() {
    if (!summary) {
      await message("暂无本次会话的同步结果。", {
        title: "同步记录",
        kind: "info",
        buttons: { ok: "知道了" },
      });
      return;
    }

    const success = summary.results.filter((result) => result.status === "success").length;
    const skipped = summary.results.filter((result) => result.status === "skipped").length;
    const failed = summary.results.filter((result) => result.status === "failed").length;
    await message(
      [
        `开始：${summary.startedAt}`,
        `完成：${summary.finishedAt}`,
        `成功：${success}`,
        `跳过：${skipped}`,
        `失败：${failed}`,
        "",
        `备份：${summary.backupPaths.length} 个`,
      ].join("\n"),
      {
        title: "同步记录",
        kind: failed > 0 ? "warning" : "info",
        buttons: { ok: "知道了" },
      },
    );
  }

  async function applyPlan() {
    if (!sourceVaultPath || operationList.length === 0) {
      setMessageText("没有选择任何同步操作。");
      return;
    }
    const downgradeWithoutConfirmation = operationList.filter((operation) => {
      const diff = findDiff(operation.targetVaultPath, operation.pluginId);
      return diff?.status === "source-older" && operation.copyPluginFiles && !operation.forceDowngrade;
    });
    if (downgradeWithoutConfirmation.length > 0) {
      setMessageText("存在会降级的插件，请先在插件详情中勾选对应插件的“允许降级”。");
      return;
    }
    if (!(await confirmDangerousOperations())) {
      return;
    }
    const confirmedClosed = await confirmObsidianClosed("apply");
    if (!confirmedClosed) {
      return;
    }

    setBusy(true);
    setSyncing(true);
    setMessageText(null);
    try {
      const running = await api.checkObsidianRunning();
      if (running) {
        setMessageText("检测到 Obsidian.exe 正在运行，请关闭后再同步。");
        return;
      }
      const nextSummary = await api.applySyncPlan({
        sourceVaultPath,
        targetVaultPaths,
        operations: operationList,
        obsidianClosedConfirmed: true,
      });
      setSummary(nextSummary);
      clearOperationSelection();
      await refreshDiff({ clearSummary: false, showEmptyMessage: false });
      await loadInventory(sourceVaultPath);
    } catch (error) {
      setMessageText(commandMessage(error));
    } finally {
      setSyncing(false);
      setBusy(false);
    }
  }

  async function restoreSelectedBackup(backup: BackupInfo): Promise<SyncSummary | null> {
    const confirmedClosed = await confirmObsidianClosed("restore");
    if (!confirmedClosed) {
      return null;
    }

    setBusy(true);
    setMessageText(null);
    try {
      const running = await api.checkObsidianRunning();
      if (running) {
        setMessageText("检测到 Obsidian.exe 正在运行，请关闭后再恢复。");
        return null;
      }
      const nextSummary = await api.restoreBackup(backup.vaultPath, backup.backupPath, true);
      setSummary(nextSummary);
      if (sourceVaultPath) {
        await loadInventory(sourceVaultPath);
        await refreshDiff({ clearSummary: false, showEmptyMessage: false });
      }
      return nextSummary;
    } catch (error) {
      setMessageText(commandMessage(error));
      return null;
    } finally {
      setBusy(false);
    }
  }

  return (
    <div
      className={`desktop-app ${workspacePage === "manage" ? "management-mode" : "sync-mode"} ${leftRailOpen ? "" : "left-rail-collapsed"} ${
        rightDrawerOpen ? "" : "right-drawer-collapsed"
      }`}
      style={layoutStyle}
    >
      <header className="app-chrome">
        <nav className="menu-row" aria-label="应用菜单">
          <div className="workspace-tabs" aria-label="工作区切换">
            <button className={workspacePage === "manage" ? "active" : ""} onClick={() => setWorkspacePage("manage")}>
              <Settings2 size={16} />
              插件管理
            </button>
            <button className={workspacePage === "sync" ? "active" : ""} onClick={() => setWorkspacePage("sync")}>
              <ArrowLeftRight size={16} />
              多库同步
            </button>
          </div>
          <span>文件(F)</span>
          <span>编辑(E)</span>
          <span>视图(V)</span>
          <span>工具(T)</span>
          <span>帮助(H)</span>
        </nav>
        <button className="history-button" onClick={() => void showSyncRecords()}>
          <History size={18} />
          同步记录
        </button>
      </header>

      <div className={`app-layout ${workspacePage === "manage" ? "management-layout" : ""}`}>
        {leftRailOpen && (
          <aside className="vault-rail" aria-label="库列表">
            <div className="rail-header">
              <h1>库列表</h1>
              <div className="rail-header-actions">
                <button className="icon-button" onClick={addVault} disabled={busy} title="添加知识库">
                  <Plus size={18} />
                </button>
                <button className="icon-button" onClick={() => setLeftRailOpen(false)} title="收起库列表">
                  <PanelLeftClose size={18} />
                </button>
              </div>
            </div>

            <div className="vault-list">
              {vaults.map((vault) => (
                <button
                  key={vault.path}
                  className={`vault-item ${sourceVaultPath === vault.path ? "active" : ""}`}
                  onClick={() => void chooseSource(vault.path)}
                  title={vault.path}
                >
                  <span className={`vault-avatar ${toneClass(vault.path)}`}>{vaultInitial(vault)}</span>
                  <span className="vault-copy">
                    <strong>{vault.name}</strong>
                    <small>{vault.source === "manual" ? "手动添加" : "Obsidian 记录"}</small>
                  </span>
                </button>
              ))}
              {vaults.length === 0 && <p className="empty-state">没有发现知识库。</p>}
            </div>

            <div className="rail-status">
              <div className="rail-status-top">
                <span className="status-dot success" />
                <strong>{busy ? "正在处理" : "本地就绪"}</strong>
                <button className="icon-button subtle" onClick={refreshVaults} disabled={busy} title="刷新知识库">
                  <RefreshCw size={16} />
                </button>
              </div>
              <small>{vaults.length} 个知识库 · {inventory?.plugins.length ?? 0} 个源库插件</small>
            </div>

            <div className="app-version">v0.1.5</div>
          </aside>
        )}

        <SidebarResizer
          side="left"
          open={leftRailOpen}
          onToggle={() => setLeftRailOpen((current) => !current)}
          onResizeStart={(event) => startSidebarResize("left", event)}
        />

        <main className={`workspace ${workspacePage === "manage" ? "management-host" : ""}`}>
          {workspacePage === "manage" ? (
            <PluginManagementPage
              vault={sourceVault}
              backups={backups}
              summary={summary}
              confirmObsidianClosed={() => confirmObsidianClosed("manage")}
              onBusyChange={setManagerBusy}
              onRefreshShared={async () => {
                if (sourceVaultPath) await loadInventory(sourceVaultPath);
              }}
              onRestoreBackup={restoreSelectedBackup}
              onSummary={setSummary}
            />
          ) : (
            <>
          <section className="flow-bar" aria-label="同步方向">
            <div className="flow-node source-node">
              <span className="node-label">源库</span>
              <div className="source-select">
                <Folder size={18} />
                <strong>{sourceVault?.name ?? "选择源库"}</strong>
              </div>
            </div>
            <ChevronRight className="flow-arrow" size={30} />
            <div className="flow-node target-node">
              <div className="node-heading">
                <span className="node-label">目标库</span>
                <strong>已选择 {targetVaultPaths.length} 个目标库</strong>
              </div>
              <div className="target-chip-list">
                {vaults
                  .filter((vault) => vault.path !== sourceVaultPath)
                  .map((vault) => {
                    const selected = targetVaultPaths.includes(vault.path);
                    return (
                      <button
                        key={vault.path}
                        className={`target-chip ${selected ? "selected" : ""}`}
                        onClick={() => void toggleTarget(vault.path)}
                        title={vault.path}
                      >
                        <span className={`vault-badge ${toneClass(vault.path)}`}>{vaultInitial(vault)}</span>
                        <span>{vault.name}</span>
                        {selected && <X size={14} />}
                      </button>
                    );
                  })}
                {vaults.filter((vault) => vault.path !== sourceVaultPath).length === 0 && (
                  <span className="quiet-text">暂无可选目标库</span>
                )}
              </div>
            </div>
            <button
              className="ghost-action refresh-diff"
              onClick={() => void refreshDiff({ clearSummary: true, showEmptyMessage: true })}
              disabled={busy || diffBusy || !sourceVaultPath || targetVaultPaths.length === 0}
            >
              <RefreshCw size={17} />
              重新扫描
            </button>
          </section>

          {messageText && (
            <div className="notice danger">
              <AlertTriangle size={17} />
              <span>{messageText}</span>
            </div>
          )}

          <section className="plugin-workspace" aria-label="插件差异">
            <div className="section-header">
              <div>
                <h2>插件同步差异</h2>
                <p>{diffBusy ? "正在比较插件文件、设置和启用状态" : `${pluginCards.length} 个插件 · 自动生成差异`}</p>
              </div>
              <StatusChip label={diffBusy ? "比较中" : "可选择"} tone={diffBusy ? "warning" : "accent"} />
            </div>

            {diffBusy && pluginCards.length === 0 ? (
              <div className="plugin-grid">
                {Array.from({ length: 6 }).map((_, index) => (
                  <div key={index} className="plugin-card skeleton-card" />
                ))}
              </div>
            ) : pluginCards.length > 0 ? (
              <div className="plugin-grid">
                {pluginCards.map((card) => (
                  <PluginCard
                    key={card.pluginId}
                    card={card}
                    onFocus={() => setFocusedPluginId(card.pluginId)}
                    onToggle={() => togglePluginSelection(card)}
                  />
                ))}
              </div>
            ) : (
              <div className="empty-panel">
                <ShieldCheck size={28} />
                <strong>{sourceVaultPath && targetVaultPaths.length > 0 ? "暂无插件差异" : "等待选择源库和目标库"}</strong>
                <span>
                  {sourceVaultPath && targetVaultPaths.length > 0
                    ? "没有可显示的插件比较结果。"
                    : "选择后会自动生成差异。"}
                </span>
              </div>
            )}
          </section>
            </>
          )}
        </main>

        {workspacePage === "sync" && (
          <SidebarResizer
            side="right"
            open={rightDrawerOpen}
            onToggle={() => setRightDrawerOpen((current) => !current)}
            onResizeStart={(event) => startSidebarResize("right", event)}
          />
        )}

        {workspacePage === "sync" && rightDrawerOpen && (
          <PluginDetailDrawer
            card={focusedCard}
            backups={backups}
            busy={busy}
            operations={operations}
            summary={summary}
            onCollapse={() => setRightDrawerOpen(false)}
            onOperationChange={updateOperation}
            onRestoreBackup={restoreSelectedBackup}
          />
        )}
      </div>

      {workspacePage === "sync" ? (
      <footer className="batch-bar">
        <div className="batch-summary">
          <span className="batch-check">
            <Check size={16} />
          </span>
          <strong>已选 {selectedPluginIds.length} 个插件</strong>
          <span>{operationList.length} 项操作</span>
          {syncing && (
            <span className="sync-indicator">
              <LoaderCircle className="spin-icon" size={14} />
              正在执行同步
            </span>
          )}
          {riskCount > 0 && <span className="risk-text">风险 {riskCount} 项</span>}
        </div>
        <div className="batch-actions">
          <button className="ghost-action" onClick={clearOperationSelection} disabled={busy || selectedPluginIds.length === 0}>
            取消选择
          </button>
          <button className="ghost-action accent" onClick={() => void previewPlan()} disabled={busy || operationList.length === 0}>
            预览变更
          </button>
          <button
            className={`primary-action ${syncing ? "syncing" : ""}`}
            onClick={applyPlan}
            disabled={busy || diffBusy || operationList.length === 0}
          >
            {syncing ? <LoaderCircle className="spin-icon" size={17} /> : <Play size={17} />}
            {syncing ? "正在同步" : "同步选中插件"}
          </button>
        </div>
      </footer>
      ) : (
        <footer className="management-footer">
          <span><ShieldCheck size={16} /> 所有写入都会先创建可恢复备份</span>
          <strong>{sourceVault?.name ?? "未选择知识库"}</strong>
        </footer>
      )}

      {closeDialogOpen && (
        <CloseIntentDialog
          busy={busy || diffBusy || managerBusy}
          operationCount={operationList.length}
          onCancel={() => setCloseDialogOpen(false)}
          onDirectClose={() => void closeWindowDirectly()}
          onHideToTray={() => void hideWindowToTray()}
        />
      )}

      {obsidianDialogAction && (
        <ObsidianClosedDialog
          action={obsidianDialogAction}
          onCancel={() => finishObsidianConfirm(false)}
          onConfirm={() => finishObsidianConfirm(true)}
        />
      )}
    </div>
  );
}

type SidebarResizerProps = {
  side: SidebarSide;
  open: boolean;
  onToggle: () => void;
  onResizeStart: (event: ReactPointerEvent<HTMLDivElement>) => void;
};

function SidebarResizer({ side, open, onToggle, onResizeStart }: SidebarResizerProps) {
  const isLeft = side === "left";
  const ToggleIcon = isLeft
    ? open
      ? PanelLeftClose
      : PanelLeftOpen
    : open
      ? PanelRightClose
      : PanelRightOpen;
  const toggleLabel = isLeft
    ? open
      ? "收起库列表"
      : "展开库列表"
    : open
      ? "收起插件详情"
      : "展开插件详情";

  return (
    <div
      className={`sidebar-resizer ${side} ${open ? "open" : "collapsed"}`}
      role={open ? "separator" : "presentation"}
      aria-label={open ? (isLeft ? "拖动调整库列表宽度" : "拖动调整插件详情宽度") : toggleLabel}
      aria-orientation="vertical"
      onPointerDown={open ? onResizeStart : undefined}
    >
      <button
        className="sidebar-toggle"
        onClick={onToggle}
        onPointerDown={(event) => event.stopPropagation()}
        aria-label={toggleLabel}
        aria-pressed={open}
        title={toggleLabel}
      >
        <ToggleIcon size={16} />
      </button>
      {open && (
        <span className="resizer-grip" aria-hidden="true">
          <GripVertical size={16} />
        </span>
      )}
    </div>
  );
}

type CloseIntentDialogProps = {
  busy: boolean;
  operationCount: number;
  onCancel: () => void;
  onDirectClose: () => void;
  onHideToTray: () => void;
};

function CloseIntentDialog({
  busy,
  operationCount,
  onCancel,
  onDirectClose,
  onHideToTray,
}: CloseIntentDialogProps) {
  return (
    <div className="close-overlay">
      <section className="close-dialog" role="dialog" aria-modal="true" aria-labelledby="close-dialog-title">
        <header className="close-dialog-header">
          <div>
            <p>关闭方式</p>
            <h2 id="close-dialog-title">要把 Obsidian Plugin Sync 放到哪里？</h2>
          </div>
          <button className="icon-button" onClick={onCancel} title="取消关闭">
            <X size={18} />
          </button>
        </header>

        <div className="close-options">
          <button className="close-option recommended" onClick={onHideToTray}>
            <span className="close-option-icon">
              <Minimize2 size={22} />
            </span>
            <span>
              <strong>隐藏到系统托盘</strong>
              <small>保留在右下角后台，稍后可以继续查看同步状态。</small>
            </span>
            <StatusChip label="推荐" tone="accent" />
          </button>

          <button className="close-option" onClick={onDirectClose}>
            <span className="close-option-icon danger">
              <Power size={22} />
            </span>
            <span>
              <strong>直接关闭软件</strong>
              <small>立即退出程序，不会自动应用当前未同步的选择。</small>
            </span>
          </button>
        </div>

        {(busy || operationCount > 0) && (
          <div className="close-dialog-note">
            {busy ? "当前仍有扫描或同步流程在执行，建议完成后再直接关闭。" : `当前有 ${operationCount} 项待同步操作。`}
          </div>
        )}

        <footer className="close-dialog-actions">
          <button className="ghost-action" onClick={onCancel}>
            取消
          </button>
        </footer>
      </section>
    </div>
  );
}

type ObsidianClosedDialogProps = {
  action: ObsidianWriteAction;
  onCancel: () => void;
  onConfirm: () => void;
};

function ObsidianClosedDialog({ action, onCancel, onConfirm }: ObsidianClosedDialogProps) {
  const [checking, setChecking] = useState(true);
  const [running, setRunning] = useState<boolean | null>(null);
  const actionLabel = action === "apply" ? "同步选中插件" : action === "restore" ? "恢复备份" : "保存单库插件变更";

  useEffect(() => {
    void refreshProcessState();
  }, []);

  async function refreshProcessState() {
    setChecking(true);
    try {
      setRunning(await api.checkObsidianRunning());
    } catch {
      setRunning(null);
    } finally {
      setChecking(false);
    }
  }

  const statusTone: ChipTone = checking ? "neutral" : running === true ? "danger" : running === false ? "success" : "warning";
  const statusLabel = checking
    ? "正在检测"
    : running === true
      ? "仍在运行"
      : running === false
        ? "未检测到进程"
        : "检测失败";
  const statusDetail = checking
    ? "正在通过系统进程列表检查 Obsidian.exe…"
    : running === true
      ? "检测到 Obsidian.exe 仍在运行。请完全退出 Obsidian 后再继续，避免写入冲突。"
      : running === false
        ? "当前未检测到 Obsidian.exe。请仍确认你已手动关闭所有 Obsidian 窗口。"
        : "无法确认进程状态。你可以重试检测，或在确认已关闭后继续。";

  const canConfirm = !checking && running !== true;

  return (
    <div className="close-overlay">
      <section
        className="close-dialog safety-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="obsidian-closed-title"
      >
        <header className="close-dialog-header">
          <div>
            <p>写入安全门闩</p>
            <h2 id="obsidian-closed-title">确认 Obsidian 已关闭</h2>
          </div>
          <button className="icon-button" onClick={onCancel} title="取消">
            <X size={18} />
          </button>
        </header>

        <div className="safety-dialog-body">
          <div className={`safety-status-card ${statusTone}`}>
            <span className={`safety-status-icon ${statusTone}`}>
              {checking ? (
                <LoaderCircle className="spin-icon" size={22} />
              ) : running === true ? (
                <AlertTriangle size={22} />
              ) : running === false ? (
                <ShieldCheck size={22} />
              ) : (
                <Info size={22} />
              )}
            </span>
            <div className="safety-status-copy">
              <div className="safety-status-title-row">
                <strong>Obsidian 进程</strong>
                <StatusChip label={statusLabel} tone={statusTone} />
              </div>
              <p>{statusDetail}</p>
            </div>
          </div>

          <div className="safety-action-card">
            <span className="close-option-icon">
              {action === "apply" ? <Play size={20} /> : action === "restore" ? <RotateCcw size={20} /> : <Settings2 size={20} />}
            </span>
            <div>
              <strong>即将执行：{actionLabel}</strong>
              <small>写入会修改目标知识库中的插件文件、设置或启用状态，并先创建本地备份。</small>
            </div>
          </div>

          <ul className="safety-checklist">
            <li>关闭所有 Obsidian 主窗口（托盘驻留也算运行中）。</li>
            <li>等待几秒后再点「重新检测」，确认状态为未检测到进程。</li>
            <li>确认后仍会由程序再次校验；若仍在运行会拦截写入。</li>
          </ul>
        </div>

        {running === true && (
          <div className="close-dialog-note">
            Obsidian 仍在运行时不能继续。请关闭后再点「重新检测」。
          </div>
        )}

        <footer className="close-dialog-actions safety-dialog-actions">
          <button className="ghost-action" onClick={onCancel}>
            取消
          </button>
          <button className="ghost-action accent" onClick={() => void refreshProcessState()} disabled={checking}>
            {checking ? <LoaderCircle className="spin-icon" size={16} /> : <RefreshCw size={16} />}
            重新检测
          </button>
          <button className="primary-action" onClick={onConfirm} disabled={!canConfirm}>
            <ShieldCheck size={17} />
            我已关闭 Obsidian，继续
          </button>
        </footer>
      </section>
    </div>
  );
}

type PluginCardProps = {
  card: PluginCardModel;
  onFocus: () => void;
  onToggle: () => void;
};

function PluginCard({ card, onFocus, onToggle }: PluginCardProps) {
  const targetBadges = card.targets.slice(0, 4);
  return (
    <article
      className={`plugin-card ${card.selected ? "selected" : ""} ${card.focused ? "focused" : ""}`}
      onClick={onFocus}
      onKeyDown={(event) => {
        if (event.key === "Enter") onFocus();
      }}
      tabIndex={0}
    >
      <div className="plugin-card-head">
        <label className="card-checkbox" onClick={(event) => event.stopPropagation()}>
          <input type="checkbox" checked={card.selected} onChange={onToggle} />
          <span />
        </label>
        <span className={`plugin-icon ${toneClass(card.pluginId)}`}>
          <Box size={28} />
        </span>
        <div className="plugin-title">
          <strong>{card.displayName}</strong>
          <small>{card.pluginId}</small>
        </div>
        <div className="target-badges" aria-label="目标库">
          {targetBadges.map(({ targetVault }) => (
            <span key={targetVault.path} className={`vault-badge ${toneClass(targetVault.path)}`} title={targetVault.name}>
              {vaultInitial(targetVault)}
            </span>
          ))}
          {card.targets.length > targetBadges.length && (
            <span className="vault-badge more">+{card.targets.length - targetBadges.length}</span>
          )}
        </div>
      </div>

      <dl className="plugin-metrics">
        <div>
          <dt>目标版本</dt>
          <dd>
            <StatusChip label={card.versionText} tone="neutral" />
          </dd>
        </div>
        <div>
          <dt>启用状态</dt>
          <dd>
            <StatusChip {...card.enabled} />
          </dd>
        </div>
        <div>
          <dt>插件文件</dt>
          <dd>
            <StatusChip {...card.files} />
          </dd>
        </div>
        <div>
          <dt>设置</dt>
          <dd>
            <StatusChip {...card.settings} />
          </dd>
        </div>
      </dl>

      <div className="card-foot">
        <StatusChip {...card.status} />
        {card.operationCount > 0 && <span className="operation-count">{card.operationCount} 个目标待同步</span>}
        {card.riskCount > 0 && <span className="risk-text">{card.riskCount} 项风险</span>}
      </div>
    </article>
  );
}

type PluginDetailDrawerProps = {
  card: PluginCardModel | null;
  backups: BackupInfo[];
  busy: boolean;
  operations: OperationMap;
  summary: SyncSummary | null;
  onCollapse: () => void;
  onOperationChange: (targetPath: string, diff: PluginDiff, patch: Partial<SelectedPluginOperation>) => void;
  onRestoreBackup: (backup: BackupInfo) => void;
};

function PluginDetailDrawer({
  card,
  backups,
  busy,
  operations,
  summary,
  onCollapse,
  onOperationChange,
  onRestoreBackup,
}: PluginDetailDrawerProps) {
  return (
    <aside className="detail-drawer" aria-label="插件详情">
      <div className="drawer-header">
        <div>
          <h2>插件详情</h2>
          <p>{card ? "目标库状态" : "等待选择插件"}</p>
        </div>
        <div className="drawer-header-actions">
          <Settings2 size={18} />
          <button className="icon-button" onClick={onCollapse} title="收起插件详情">
            <PanelRightClose size={18} />
          </button>
        </div>
      </div>

      {card ? (
        <>
          <section className="drawer-plugin">
            <span className={`plugin-icon large ${toneClass(card.pluginId)}`}>
              <Box size={34} />
            </span>
            <div>
              <h3>{card.displayName}</h3>
              <p>源状态：{card.sourcePlugin?.enabled ? "已启用" : "已禁用"} · {card.versionText}</p>
            </div>
          </section>

          <section className="drawer-section">
            <div className="drawer-section-title">
              <ShieldCheck size={16} />
              目标库状态 ({card.targets.length})
            </div>
            <div className="target-table">
              <div className="target-table-head">
                <span>目标库</span>
                <span>版本</span>
                <span>启用</span>
                <span>文件</span>
                <span>设置</span>
                <span>风险</span>
              </div>
              {card.targets.map(({ targetVault, diff }) => (
                <TargetStatusRow
                  key={targetVault.path}
                  targetVault={targetVault}
                  diff={diff}
                  operation={operations[operationKey(targetVault.path, diff.pluginId)]}
                  onOperationChange={onOperationChange}
                />
              ))}
            </div>
          </section>

          {card.warnings.length > 0 && (
            <section className="drawer-section warning-section">
              <div className="drawer-section-title">
                <AlertTriangle size={16} />
                提示
              </div>
              {Array.from(new Set(card.warnings)).map((warning) => (
                <p key={warning}>{warning}</p>
              ))}
            </section>
          )}
        </>
      ) : (
        <div className="drawer-empty">
          <Info size={26} />
          <strong>未选择插件</strong>
          <span>插件卡片会在生成差异后显示。</span>
        </div>
      )}

      <section className="drawer-section">
        <div className="drawer-section-title">
          <RotateCcw size={16} />
          从备份恢复
        </div>
        <div className="backup-list">
          {backups.slice(0, 4).map((backup) => (
            <div key={backup.backupPath} className="backup-row">
              <span>
                <strong>{backup.createdAt}</strong>
                <small>{backup.backupPath}</small>
              </span>
              <button className="ghost-action mini" onClick={() => onRestoreBackup(backup)} disabled={busy}>
                恢复
              </button>
            </div>
          ))}
          {backups.length === 0 && <p className="empty-state">当前源库没有可用备份。</p>}
        </div>
      </section>

      <section className="drawer-section">
        <div className="drawer-section-title">
          <DatabaseBackup size={16} />
          执行结果
        </div>
        {summary ? (
          <div className="result-list">
            {summary.results.slice(0, 8).map((result, index) => (
              <div key={`${result.targetVaultPath}-${result.pluginId}-${result.action}-${index}`} className="result-row">
                <StatusChip label={result.status} tone={resultTone(result.status)} />
                <span>{result.message}</span>
              </div>
            ))}
            {summary.results.length > 8 && <p className="empty-state">还有 {summary.results.length - 8} 条结果。</p>}
          </div>
        ) : (
          <p className="empty-state">同步或恢复后显示结果。</p>
        )}
      </section>
    </aside>
  );
}

type TargetStatusRowProps = {
  targetVault: Vault;
  diff: PluginDiff;
  operation: SelectedPluginOperation | undefined;
  onOperationChange: (targetPath: string, diff: PluginDiff, patch: Partial<SelectedPluginOperation>) => void;
};

function TargetStatusRow({ targetVault, diff, operation, onOperationChange }: TargetStatusRowProps) {
  const blocked = isBlockedDiff(diff);
  const sourceBackedDisabled = blocked || diff.status === "target-only";
  const canSyncSettings = sourceBackedDisabled || !diff.sourcePlugin?.hasDataJson;
  const riskyDowngrade = diff.status === "source-older";
  const files = filesSummary([{ targetVault, diff, operation }]);
  const settings = settingsSummary([{ targetVault, diff, operation }]);
  const enabled = enabledSummary([{ targetVault, diff, operation }]);
  const risk =
    diff.status === "source-older"
      ? ({ label: "降级", tone: "danger" } satisfies DimensionSummary)
      : diff.status === "target-only"
        ? ({ label: "可删除", tone: "warning" } satisfies DimensionSummary)
        : ({ label: "无", tone: "neutral" } satisfies DimensionSummary);

  return (
    <div className="target-row">
      <div className="target-row-main">
        <span className="target-name-cell">
          <span className={`vault-badge ${toneClass(targetVault.path)}`}>{vaultInitial(targetVault)}</span>
          <strong title={targetVault.path}>{targetVault.name}</strong>
        </span>
        <span>{versionLabel(diff.targetPlugin?.version)}</span>
        <StatusChip {...enabled} />
        <StatusChip {...files} />
        <StatusChip {...settings} />
        <StatusChip {...risk} />
      </div>
      <div className="operation-toggles">
        <label>
          <input
            type="checkbox"
            disabled={sourceBackedDisabled}
            checked={operation?.copyPluginFiles ?? false}
            onChange={(event) =>
              onOperationChange(targetVault.path, diff, { copyPluginFiles: event.target.checked })
            }
          />
          文件
        </label>
        <label>
          <input
            type="checkbox"
            disabled={canSyncSettings}
            checked={operation?.syncDataJson ?? false}
            onChange={(event) => onOperationChange(targetVault.path, diff, { syncDataJson: event.target.checked })}
          />
          设置
        </label>
        <label>
          <input
            type="checkbox"
            disabled={sourceBackedDisabled}
            checked={operation?.syncEnabledState ?? false}
            onChange={(event) =>
              onOperationChange(targetVault.path, diff, { syncEnabledState: event.target.checked })
            }
          />
          启用
        </label>
        <label className="danger-option">
          <input
            type="checkbox"
            disabled={blocked || diff.status !== "target-only"}
            checked={operation?.deleteTargetPlugin ?? false}
            onChange={(event) =>
              onOperationChange(targetVault.path, diff, { deleteTargetPlugin: event.target.checked })
            }
          />
          删除
        </label>
        <label className="danger-option">
          <input
            type="checkbox"
            disabled={blocked || !riskyDowngrade}
            checked={operation?.forceDowngrade ?? false}
            onChange={(event) => onOperationChange(targetVault.path, diff, { forceDowngrade: event.target.checked })}
          />
          允许降级
        </label>
      </div>
      {diff.warnings.length > 0 && <p className="row-warning">{diff.warnings.join("；")}</p>}
    </div>
  );
}

export default App;
