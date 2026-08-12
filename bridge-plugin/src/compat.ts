import { type App, type PluginManifest } from "obsidian";
import { BRIDGE_PLUGIN_ID, isValidPluginId } from "./protocol.ts";

type InternalPluginRegistry = {
  manifests?: Record<string, PluginManifest>;
  plugins?: Record<string, unknown>;
};

type InternalSettingTab = {
  id?: string;
  name?: string;
  containerEl?: HTMLElement;
  plugin?: { manifest?: PluginManifest };
  display?: () => void | Promise<void>;
};

type InternalSettingsManager = {
  open?: () => void | Promise<void>;
  openTabById?: (id: string) => void | Promise<void>;
  activeTab?: InternalSettingTab;
  pluginTabs?: InternalSettingTab[] | Record<string, InternalSettingTab>;
  settingTabs?: InternalSettingTab[] | Record<string, InternalSettingTab>;
};

export type BridgePluginCandidate = {
  id: string;
  name: string;
  version: string | null;
  dir: string;
};

export function getPluginCandidates(app: App): BridgePluginCandidate[] {
  const registry = getPluginRegistry(app);
  return Object.values(registry.manifests ?? {})
    .filter((manifest) => manifest.id !== BRIDGE_PLUGIN_ID)
    .filter((manifest) => Boolean(registry.plugins?.[manifest.id]))
    .filter((manifest) => isValidPluginId(manifest.id) && Boolean(manifest.dir))
    .map((manifest) => ({
      id: manifest.id,
      name: manifest.name || manifest.id,
      version: manifest.version || null,
      dir: manifest.dir as string,
    }))
    .sort((left, right) => left.name.localeCompare(right.name));
}

export function resolvePluginCandidate(app: App, pluginId: string): BridgePluginCandidate {
  if (!isValidPluginId(pluginId) || pluginId === BRIDGE_PLUGIN_ID) throw new Error("插件 ID 无效");
  const candidate = getPluginCandidates(app).find((item) => item.id === pluginId);
  if (!candidate) throw new Error("插件未安装、未启用或没有可用的运行时实例");
  return candidate;
}

export async function openSettingsManager(app: App): Promise<void> {
  const manager = getSettingsManager(app);
  if (typeof manager.open !== "function") throw new Error("当前 Obsidian 版本不支持打开设置管理器");
  await Promise.resolve(manager.open());
  await waitFrame();
}

export async function renderPluginSettingsTab(app: App, pluginId: string): Promise<HTMLElement> {
  const manager = getSettingsManager(app);
  if (typeof manager.openTabById !== "function") throw new Error("当前 Obsidian 版本不支持按插件打开设置页");
  const before = findTab(manager, pluginId);
  const alreadyActive = tabPluginId(manager.activeTab) === pluginId;
  if (alreadyActive && before?.display && before.containerEl) {
    before.containerEl.replaceChildren();
    await Promise.resolve(before.display());
  } else {
    await Promise.resolve(manager.openTabById(pluginId));
  }
  await waitFrame();
  await waitFrame();
  const tab = findTab(manager, pluginId) ?? manager.activeTab;
  if (tabPluginId(tab) !== pluginId || !tab?.containerEl) {
    throw new Error("未找到目标插件的真实设置页；插件可能没有注册设置标签");
  }
  return tab.containerEl;
}

export async function openPluginSettingsTab(app: App, pluginId: string): Promise<void> {
  if (pluginId !== BRIDGE_PLUGIN_ID) resolvePluginCandidate(app, pluginId);
  await openSettingsManager(app);
  const manager = getSettingsManager(app);
  if (typeof manager.openTabById !== "function") throw new Error("当前 Obsidian 版本不支持按插件打开设置页");
  await Promise.resolve(manager.openTabById(pluginId));
}

function getPluginRegistry(app: App): InternalPluginRegistry {
  const registry = (app as App & { plugins?: InternalPluginRegistry }).plugins;
  if (!registry?.manifests || !registry.plugins) throw new Error("当前 Obsidian 版本未暴露插件注册表");
  return registry;
}

function getSettingsManager(app: App): InternalSettingsManager {
  const manager = (app as App & { setting?: InternalSettingsManager }).setting;
  if (!manager) throw new Error("当前 Obsidian 版本未暴露设置管理器");
  return manager;
}

function findTab(manager: InternalSettingsManager, pluginId: string): InternalSettingTab | undefined {
  const tabs = [...tabValues(manager.pluginTabs), ...tabValues(manager.settingTabs)];
  return tabs.find((tab) => tabPluginId(tab) === pluginId);
}

function tabValues(value: InternalSettingTab[] | Record<string, InternalSettingTab> | undefined): InternalSettingTab[] {
  if (!value) return [];
  return Array.isArray(value) ? value : Object.values(value);
}

function tabPluginId(tab: InternalSettingTab | undefined): string | null {
  return tab?.plugin?.manifest?.id ?? tab?.id ?? null;
}

function waitFrame(): Promise<void> {
  return new Promise((resolve) => window.requestAnimationFrame(() => resolve()));
}
