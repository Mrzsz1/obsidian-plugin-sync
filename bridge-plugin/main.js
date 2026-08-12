"use strict";
var __defProp = Object.defineProperty;
var __getOwnPropDesc = Object.getOwnPropertyDescriptor;
var __getOwnPropNames = Object.getOwnPropertyNames;
var __hasOwnProp = Object.prototype.hasOwnProperty;
var __export = (target, all) => {
  for (var name in all)
    __defProp(target, name, { get: all[name], enumerable: true });
};
var __copyProps = (to, from, except, desc) => {
  if (from && typeof from === "object" || typeof from === "function") {
    for (let key of __getOwnPropNames(from))
      if (!__hasOwnProp.call(to, key) && key !== except)
        __defProp(to, key, { get: () => from[key], enumerable: !(desc = __getOwnPropDesc(from, key)) || desc.enumerable });
  }
  return to;
};
var __toCommonJS = (mod) => __copyProps(__defProp({}, "__esModule", { value: true }), mod);

// src/main.ts
var main_exports = {};
__export(main_exports, {
  default: () => ObsidianPluginSyncBridge
});
module.exports = __toCommonJS(main_exports);
var import_obsidian2 = require("obsidian");

// src/batch.ts
async function runSequentialBatch(pluginIds, capture, shouldCancel, onProgress) {
  const results = [];
  for (const pluginId of pluginIds) {
    if (shouldCancel()) {
      results.push({ pluginId, status: "cancelled", message: "\u5DF2\u5728\u5F00\u59CB\u6B64\u63D2\u4EF6\u524D\u53D6\u6D88" });
      break;
    }
    let result;
    try {
      await capture(pluginId);
      result = { pluginId, status: "success", message: "\u5DF2\u7F13\u5B58\u8FD0\u884C\u65F6\u8BBE\u7F6E\u7ED3\u6784" };
    } catch (error) {
      result = {
        pluginId,
        status: "failed",
        message: error instanceof Error ? error.message : String(error)
      };
    }
    results.push(result);
    onProgress?.(results.length, pluginIds.length, result);
  }
  return { cancelled: shouldCancel(), results };
}

// src/cache.ts
var import_obsidian = require("obsidian");

// src/protocol.ts
var BRIDGE_PLUGIN_ID = "obsidian-plugin-sync-bridge";
var BRIDGE_VERSION = "0.1.0";
var BRIDGE_PROTOCOL_VERSION = 1;
var BRIDGE_CACHE_VERSION = 1;
var BRIDGE_URI_ACTION = BRIDGE_PLUGIN_ID;
var PLUGIN_ID_PATTERN = /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/;
var REQUEST_KEYS = /* @__PURE__ */ new Set(["action", "op", "plugin", "protocol", "vault"]);
var SECRET_PATTERN = /(?:^|[\s._-])(api[\s._-]*key|access[\s._-]*token|refresh[\s._-]*token|token|secret|password|passphrase|credential|authorization)(?:$|[\s._-])/i;
function isValidPluginId(value) {
  return PLUGIN_ID_PATTERN.test(value) && value !== "." && value !== "..";
}
function isSensitiveLabel(value) {
  return SECRET_PATTERN.test(` ${value.trim()} `);
}
function sanitizeStructureText(value, maxLength = 500) {
  if (typeof value !== "string") return null;
  const normalized = value.replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/g, "").trim();
  if (!normalized) return null;
  return normalized.slice(0, maxLength);
}
function fnv1a64(bytes) {
  let hash = 0xcbf29ce484222325n;
  for (const byte of bytes) {
    hash ^= BigInt(byte);
    hash = BigInt.asUintN(64, hash * 0x100000001b3n);
  }
  return `${bytes.length}-${hash.toString(16).padStart(16, "0")}`;
}
function parseBridgeRequest(data) {
  for (const key of Object.keys(data)) {
    if (!REQUEST_KEYS.has(key)) throw new Error(`unsupported request field: ${key}`);
  }
  if (data.protocol !== String(BRIDGE_PROTOCOL_VERSION)) throw new Error("unsupported bridge protocol");
  if (data.op !== "capture" && data.op !== "open-settings") throw new Error("unsupported bridge operation");
  if (!isValidPluginId(data.plugin ?? "")) throw new Error("invalid plugin id");
  const vaultName = sanitizeStructureText(data.vault, 255);
  if (!vaultName) throw new Error("invalid vault name");
  return { operation: data.op, vaultName, pluginId: data.plugin };
}
function validateBridgeRequestVault(request, currentVaultName) {
  const normalizedCurrentVault = sanitizeStructureText(currentVaultName, 255);
  if (!normalizedCurrentVault || request.vaultName !== normalizedCurrentVault) {
    throw new Error("URI \u6307\u5B9A\u7684\u77E5\u8BC6\u5E93\u4E0E\u5F53\u524D\u77E5\u8BC6\u5E93\u4E0D\u4E00\u81F4");
  }
}
function cacheFileName(pluginId) {
  if (!isValidPluginId(pluginId)) throw new Error("invalid plugin id");
  return `${pluginId}.json`;
}

// src/cache.ts
function bridgeRoot(pluginDir) {
  return (0, import_obsidian.normalizePath)(pluginDir);
}
async function writeRuntimeStatus(app, pluginDir, obsidianVersion, locale) {
  const status = {
    bridgeVersion: BRIDGE_VERSION,
    protocolVersion: BRIDGE_PROTOCOL_VERSION,
    cacheVersion: BRIDGE_CACHE_VERSION,
    obsidianVersion,
    locale,
    vaultName: app.vault.getName(),
    updatedAt: (/* @__PURE__ */ new Date()).toISOString()
  };
  await writeJsonAtomic(app, (0, import_obsidian.normalizePath)(`${bridgeRoot(pluginDir)}/bridge-status.json`), status);
}
async function buildFingerprint(app, candidate, obsidianVersion, locale) {
  return {
    pluginId: candidate.id,
    pluginVersion: candidate.version,
    pluginMainHash: await fileFingerprint(app, (0, import_obsidian.normalizePath)(`${candidate.dir}/main.js`)),
    obsidianVersion,
    locale,
    protocolVersion: BRIDGE_PROTOCOL_VERSION,
    configurationHash: await fileFingerprint(app, (0, import_obsidian.normalizePath)(`${candidate.dir}/data.json`))
  };
}
async function writeSnapshot(app, pluginDir, fingerprint, snapshot) {
  const cache = {
    cacheVersion: BRIDGE_CACHE_VERSION,
    capturedAt: (/* @__PURE__ */ new Date()).toISOString(),
    fingerprint,
    snapshot
  };
  const path = (0, import_obsidian.normalizePath)(`${bridgeRoot(pluginDir)}/cache/v${BRIDGE_CACHE_VERSION}/${cacheFileName(snapshot.pluginId)}`);
  await writeJsonAtomic(app, path, cache);
}
async function writeBatchReport(app, pluginDir, report) {
  const path = (0, import_obsidian.normalizePath)(`${bridgeRoot(pluginDir)}/cache/v${BRIDGE_CACHE_VERSION}/batch-latest.json`);
  await writeJsonAtomic(app, path, report);
}
function currentLocale() {
  return localStorage.getItem("language") ?? document.documentElement.lang ?? "unknown";
}
async function fileFingerprint(app, path) {
  if (!await app.vault.adapter.exists(path)) return "missing";
  const bytes = new Uint8Array(await app.vault.adapter.readBinary(path));
  return fnv1a64(bytes);
}
async function writeJsonAtomic(app, path, value) {
  const parent = path.slice(0, path.lastIndexOf("/"));
  await ensureDirectory(app, parent);
  const temporary = `${path}.ops-temp`;
  try {
    await app.vault.adapter.write(temporary, JSON.stringify(value, null, 2));
    if (await app.vault.adapter.exists(path)) await app.vault.adapter.remove(path);
    await app.vault.adapter.rename(temporary, path);
  } finally {
    if (await app.vault.adapter.exists(temporary)) await app.vault.adapter.remove(temporary);
  }
}
async function ensureDirectory(app, path) {
  const segments = (0, import_obsidian.normalizePath)(path).split("/").filter(Boolean);
  let current = "";
  for (const segment of segments) {
    current = current ? `${current}/${segment}` : segment;
    if (!await app.vault.adapter.exists(current)) await app.vault.adapter.mkdir(current);
  }
}

// src/compat.ts
function getPluginCandidates(app) {
  const registry = getPluginRegistry(app);
  return Object.values(registry.manifests ?? {}).filter((manifest) => manifest.id !== BRIDGE_PLUGIN_ID).filter((manifest) => Boolean(registry.plugins?.[manifest.id])).filter((manifest) => isValidPluginId(manifest.id) && Boolean(manifest.dir)).map((manifest) => ({
    id: manifest.id,
    name: manifest.name || manifest.id,
    version: manifest.version || null,
    dir: manifest.dir
  })).sort((left, right) => left.name.localeCompare(right.name));
}
function resolvePluginCandidate(app, pluginId) {
  if (!isValidPluginId(pluginId) || pluginId === BRIDGE_PLUGIN_ID) throw new Error("\u63D2\u4EF6 ID \u65E0\u6548");
  const candidate = getPluginCandidates(app).find((item) => item.id === pluginId);
  if (!candidate) throw new Error("\u63D2\u4EF6\u672A\u5B89\u88C5\u3001\u672A\u542F\u7528\u6216\u6CA1\u6709\u53EF\u7528\u7684\u8FD0\u884C\u65F6\u5B9E\u4F8B");
  return candidate;
}
async function openSettingsManager(app) {
  const manager = getSettingsManager(app);
  if (typeof manager.open !== "function") throw new Error("\u5F53\u524D Obsidian \u7248\u672C\u4E0D\u652F\u6301\u6253\u5F00\u8BBE\u7F6E\u7BA1\u7406\u5668");
  await Promise.resolve(manager.open());
  await waitFrame();
}
async function renderPluginSettingsTab(app, pluginId) {
  const manager = getSettingsManager(app);
  if (typeof manager.openTabById !== "function") throw new Error("\u5F53\u524D Obsidian \u7248\u672C\u4E0D\u652F\u6301\u6309\u63D2\u4EF6\u6253\u5F00\u8BBE\u7F6E\u9875");
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
    throw new Error("\u672A\u627E\u5230\u76EE\u6807\u63D2\u4EF6\u7684\u771F\u5B9E\u8BBE\u7F6E\u9875\uFF1B\u63D2\u4EF6\u53EF\u80FD\u6CA1\u6709\u6CE8\u518C\u8BBE\u7F6E\u6807\u7B7E");
  }
  return tab.containerEl;
}
async function openPluginSettingsTab(app, pluginId) {
  if (pluginId !== BRIDGE_PLUGIN_ID) resolvePluginCandidate(app, pluginId);
  await openSettingsManager(app);
  const manager = getSettingsManager(app);
  if (typeof manager.openTabById !== "function") throw new Error("\u5F53\u524D Obsidian \u7248\u672C\u4E0D\u652F\u6301\u6309\u63D2\u4EF6\u6253\u5F00\u8BBE\u7F6E\u9875");
  await Promise.resolve(manager.openTabById(pluginId));
}
function getPluginRegistry(app) {
  const registry = app.plugins;
  if (!registry?.manifests || !registry.plugins) throw new Error("\u5F53\u524D Obsidian \u7248\u672C\u672A\u66B4\u9732\u63D2\u4EF6\u6CE8\u518C\u8868");
  return registry;
}
function getSettingsManager(app) {
  const manager = app.setting;
  if (!manager) throw new Error("\u5F53\u524D Obsidian \u7248\u672C\u672A\u66B4\u9732\u8BBE\u7F6E\u7BA1\u7406\u5668");
  return manager;
}
function findTab(manager, pluginId) {
  const tabs = [...tabValues(manager.pluginTabs), ...tabValues(manager.settingTabs)];
  return tabs.find((tab) => tabPluginId(tab) === pluginId);
}
function tabValues(value) {
  if (!value) return [];
  return Array.isArray(value) ? value : Object.values(value);
}
function tabPluginId(tab) {
  return tab?.plugin?.manifest?.id ?? tab?.id ?? null;
}
function waitFrame() {
  return new Promise((resolve) => window.requestAnimationFrame(() => resolve()));
}

// src/recorder.ts
var VALUE_COMPONENTS = [
  ["addToggle", "toggle"],
  ["addText", "text"],
  ["addSearch", "text"],
  ["addTextArea", "textarea"],
  ["addDropdown", "dropdown"],
  ["addColorPicker", "color"],
  ["addSlider", "slider"]
];
var ACTION_COMPONENTS = ["addButton", "addExtraButton"];
var RuntimeRecorder = class {
  rows = /* @__PURE__ */ new Map();
  orderedRows = [];
  touch(setting) {
    const key = setting;
    const existing = this.rows.get(key);
    if (existing) return existing;
    const row = {
      setting,
      order: this.orderedRows.length,
      control: "unsupported",
      options: [],
      placeholder: null,
      min: null,
      max: null,
      step: null,
      disabled: false,
      heading: false,
      hasValueControl: false,
      hasAction: false
    };
    this.rows.set(key, row);
    this.orderedRows.push(row);
    return row;
  }
  setHeading(setting) {
    const row = this.touch(setting);
    row.heading = true;
    row.control = "heading";
  }
  setDisabled(setting, disabled) {
    this.touch(setting).disabled = disabled;
  }
  captureValueComponent(setting, control, component) {
    const row = this.touch(setting);
    if (!row.hasValueControl) row.control = control;
    row.hasValueControl = true;
    const record = component;
    const input = record.inputEl;
    const select = record.selectEl;
    const slider = record.sliderEl;
    const baseDisabled = record.disabled === true;
    if (input) {
      row.placeholder = sanitizeStructureText(input.placeholder, 200);
      row.disabled ||= baseDisabled || input.disabled;
      if (input.type === "password") row.control = "password";
      if (input.type === "number") row.control = "number";
    } else if (select) {
      row.disabled ||= baseDisabled || select.disabled;
      row.options = Array.from(select.options).slice(0, 500).map((option) => ({
        value: sanitizeStructureText(option.value, 500) ?? "",
        label: sanitizeStructureText(option.textContent, 500) ?? option.value
      }));
    } else if (slider) {
      row.disabled ||= baseDisabled || slider.disabled;
      row.min = finiteNumber(slider.min);
      row.max = finiteNumber(slider.max);
      row.step = slider.step === "any" ? null : finiteNumber(slider.step);
    } else {
      row.disabled ||= baseDisabled;
    }
  }
  captureActionComponent(setting, component) {
    const row = this.touch(setting);
    row.hasAction = true;
    const record = component;
    const button = record.buttonEl;
    row.disabled ||= record.disabled === true || button?.disabled === true;
  }
  finish(container, pagePath) {
    const fields = [];
    const warnings = [];
    const standardRows = /* @__PURE__ */ new Set();
    let groupTitle = null;
    for (const row of this.orderedRows) {
      const element = row.setting.settingEl;
      if (!element || !container.contains(element)) continue;
      standardRows.add(element);
      const name = sanitizeStructureText(row.setting.nameEl?.textContent, 500) ?? `\u672A\u547D\u540D\u8BBE\u7F6E ${fields.length + 1}`;
      const sensitive = isSensitiveLabel(name);
      const heading = row.heading || element.classList.contains("setting-item-heading");
      const actionOnly = row.hasAction && !row.hasValueControl;
      const control = sensitive && (row.control === "text" || row.control === "textarea") ? "password" : heading ? "heading" : row.control;
      fields.push({
        pagePath: [...pagePath],
        groupTitle,
        order: fields.length,
        name,
        description: sensitive ? null : sanitizeStructureText(row.setting.descEl?.textContent, 1e3),
        control,
        options: sensitive ? [] : row.options,
        placeholder: sensitive ? null : row.placeholder,
        min: row.min,
        max: row.max,
        step: row.step,
        disabled: row.disabled || element.getAttribute("aria-disabled") === "true",
        visible: isElementVisible(element),
        action: actionOnly,
        confidence: "exact"
      });
      if (heading) groupTitle = name;
    }
    const fallback = captureCustomControls(container, standardRows, pagePath, fields.length);
    if (fallback.length > 0) {
      warnings.push(`\u53D1\u73B0 ${fallback.length} \u4E2A\u81EA\u5B9A\u4E49 DOM \u63A7\u4EF6\uFF1B\u4EC5\u7F13\u5B58\u4F4E\u7F6E\u4FE1\u5EA6\u7ED3\u6784\uFF0C\u4E0D\u63A8\u65AD\u5199\u5165\u8DEF\u5F84`);
      fields.push(...fallback);
    }
    return { fields, warnings };
  }
};
async function captureWithSettingInstrumentation(settingPrototype, render, pagePath = []) {
  const recorder = new RuntimeRecorder();
  const patches = [];
  const prototype = settingPrototype;
  patchAfter(prototype, "setName", patches, (setting) => recorder.touch(setting));
  patchAfter(prototype, "setDesc", patches, (setting) => recorder.touch(setting));
  patchAfter(prototype, "setHeading", patches, (setting) => recorder.setHeading(setting));
  patchAfter(prototype, "setDisabled", patches, (setting, args) => {
    recorder.setDisabled(setting, args[0] === true);
  });
  for (const [method, control] of VALUE_COMPONENTS) {
    patchComponent(prototype, method, patches, (setting, component) => {
      recorder.captureValueComponent(setting, control, component);
    });
  }
  for (const method of ACTION_COMPONENTS) {
    patchComponent(prototype, method, patches, (setting, component) => {
      recorder.captureActionComponent(setting, component);
    });
  }
  try {
    const container = await render();
    return recorder.finish(container, pagePath);
  } finally {
    for (const patch of patches.reverse()) {
      Object.defineProperty(prototype, patch.name, patch.descriptor);
    }
  }
}
function patchAfter(prototype, name, patches, after) {
  const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
  if (!descriptor || typeof descriptor.value !== "function") return;
  const original = descriptor.value;
  patches.push({ name, descriptor });
  Object.defineProperty(prototype, name, {
    ...descriptor,
    value: function patchedMethod(...args) {
      const result = original.apply(this, args);
      after(this, args);
      return result;
    }
  });
}
function patchComponent(prototype, name, patches, capture) {
  const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
  if (!descriptor || typeof descriptor.value !== "function") return;
  const original = descriptor.value;
  patches.push({ name, descriptor });
  Object.defineProperty(prototype, name, {
    ...descriptor,
    value: function patchedComponentMethod(...args) {
      const callback = args[0];
      if (typeof callback !== "function") return original.apply(this, args);
      const wrapped = (component) => {
        try {
          return callback(component);
        } finally {
          capture(this, component);
        }
      };
      return original.apply(this, [wrapped, ...args.slice(1)]);
    }
  });
}
function captureCustomControls(container, standardRows, pagePath, startOrder) {
  const fields = [];
  const controls = container.querySelectorAll("input, select, textarea, button");
  for (const control of Array.from(controls)) {
    const standard = Array.from(standardRows).some((row2) => row2.contains(control));
    if (standard) continue;
    const row = control.closest(".setting-item") ?? control.parentElement;
    const label = sanitizeStructureText(
      control.getAttribute("aria-label") ?? row?.querySelector(".setting-item-name, label")?.textContent ?? control.getAttribute("placeholder"),
      500
    ) ?? `\u81EA\u5B9A\u4E49\u63A7\u4EF6 ${fields.length + 1}`;
    const sensitive = isSensitiveLabel(label) || control.getAttribute("type") === "password";
    const { control: kind, action } = domControlKind(control, sensitive);
    const tagName = control.tagName.toLowerCase();
    const select = tagName === "select" ? control : null;
    const input = tagName === "input" ? control : null;
    fields.push({
      pagePath: [...pagePath],
      groupTitle: nearestHeading(control, container),
      order: startOrder + fields.length,
      name: label,
      description: null,
      control: kind,
      options: sensitive || !select ? [] : Array.from(select.options).slice(0, 500).map((option) => ({
        value: sanitizeStructureText(option.value, 500) ?? "",
        label: sanitizeStructureText(option.textContent, 500) ?? option.value
      })),
      placeholder: sensitive ? null : sanitizeStructureText(control.getAttribute("placeholder"), 200),
      min: input?.type === "range" ? finiteNumber(input.min) : null,
      max: input?.type === "range" ? finiteNumber(input.max) : null,
      step: input?.type === "range" && input.step !== "any" ? finiteNumber(input.step) : null,
      disabled: "disabled" in control && Boolean(control.disabled),
      visible: isElementVisible(control),
      action,
      confidence: "fallback"
    });
  }
  return fields;
}
function domControlKind(control, sensitive) {
  const tagName = control.tagName.toLowerCase();
  if (tagName === "button") return { control: "unsupported", action: true };
  if (tagName === "textarea") return { control: sensitive ? "password" : "textarea", action: false };
  if (tagName === "select") return { control: "dropdown", action: false };
  if (tagName === "input") {
    const input = control;
    if (sensitive || input.type === "password") return { control: "password", action: false };
    if (input.type === "checkbox") return { control: "toggle", action: false };
    if (input.type === "range") return { control: "slider", action: false };
    if (input.type === "number") return { control: "number", action: false };
    if (input.type === "color") return { control: "color", action: false };
    return { control: "text", action: false };
  }
  return { control: "unsupported", action: false };
}
function nearestHeading(control, container) {
  let heading = null;
  const following = control.ownerDocument.defaultView?.Node.DOCUMENT_POSITION_FOLLOWING ?? 4;
  for (const candidate of Array.from(container.querySelectorAll("h1, h2, h3, h4, .setting-item-heading"))) {
    if (candidate.compareDocumentPosition(control) & following) {
      heading = sanitizeStructureText(candidate.textContent, 500);
    }
  }
  return heading;
}
function finiteNumber(value) {
  if (value === null || value === void 0 || value === "") return null;
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : null;
}
function isElementVisible(element) {
  if (element.hidden || element.getAttribute("aria-hidden") === "true") return false;
  if (element.classList.contains("is-hidden") || element.style.display === "none") return false;
  if (typeof window !== "undefined" && element.isConnected) {
    const style = window.getComputedStyle(element);
    if (style.display === "none" || style.visibility === "hidden") return false;
  }
  return true;
}

// src/main.ts
var DEFAULT_PREFERENCES = { selectedPluginId: "" };
var ObsidianPluginSyncBridge = class extends import_obsidian2.Plugin {
  preferences = DEFAULT_PREFERENCES;
  async onload() {
    this.preferences = Object.assign({}, DEFAULT_PREFERENCES, await this.loadData());
    await this.writeStatus();
    this.addSettingTab(new BridgeSettingsTab(this.app, this));
    this.addCommand({
      id: "open-bridge-settings",
      name: "\u6253\u5F00\u63D2\u4EF6\u540C\u6B65 Bridge \u63A7\u5236\u53F0",
      callback: () => void this.openBridgeSettings()
    });
    this.registerObsidianProtocolHandler(BRIDGE_URI_ACTION, async (data) => {
      try {
        const request = parseBridgeRequest(data);
        validateBridgeRequestVault(request, this.app.vault.getName());
        if (request.operation === "capture") {
          await this.capturePlugin(request.pluginId);
          new import_obsidian2.Notice(`\u5DF2\u7F13\u5B58 ${request.pluginId} \u7684\u8FD0\u884C\u65F6\u8BBE\u7F6E\u7ED3\u6784`);
        } else {
          await this.openPluginSettings(request.pluginId);
        }
      } catch (error) {
        new import_obsidian2.Notice(`Bridge \u8BF7\u6C42\u5931\u8D25\uFF1A${error instanceof Error ? error.message : String(error)}`, 8e3);
      }
    });
  }
  async savePreferences() {
    await this.saveData(this.preferences);
  }
  candidates() {
    return getPluginCandidates(this.app);
  }
  async capturePlugin(pluginId) {
    const candidate = resolvePluginCandidate(this.app, pluginId);
    await openSettingsManager(this.app);
    const capture = await captureWithSettingInstrumentation(
      import_obsidian2.Setting.prototype,
      () => renderPluginSettingsTab(this.app, pluginId)
    );
    const snapshot = {
      protocolVersion: BRIDGE_PROTOCOL_VERSION,
      pluginId,
      pluginVersion: candidate.version,
      fields: capture.fields,
      warnings: [
        "\u8FD0\u884C\u76EE\u6807\u8BBE\u7F6E\u9875\u53EF\u80FD\u89E6\u53D1\u63D2\u4EF6\u81EA\u8EAB\u7684\u7F51\u7EDC\u3001\u6587\u4EF6\u626B\u63CF\u6216\u76D1\u542C\u5668\u526F\u4F5C\u7528\uFF1BBridge \u672A\u8C03\u7528\u4EFB\u4F55\u8BBE\u7F6E\u52A8\u4F5C",
        ...capture.warnings
      ]
    };
    const locale = currentLocale();
    const fingerprint = await buildFingerprint(this.app, candidate, import_obsidian2.apiVersion, locale);
    await writeSnapshot(this.app, this.manifest.dir ?? ".obsidian/plugins/obsidian-plugin-sync-bridge", fingerprint, snapshot);
    await this.writeStatus();
  }
  async openPluginSettings(pluginId) {
    await openPluginSettingsTab(this.app, pluginId);
  }
  async openBridgeSettings() {
    await openPluginSettingsTab(this.app, this.manifest.id);
  }
  async writeStatus() {
    await writeRuntimeStatus(
      this.app,
      this.manifest.dir ?? ".obsidian/plugins/obsidian-plugin-sync-bridge",
      import_obsidian2.apiVersion,
      currentLocale()
    );
  }
};
var BridgeSettingsTab = class extends import_obsidian2.PluginSettingTab {
  constructor(app, bridge) {
    super(app, bridge);
    this.bridge = bridge;
  }
  bridge;
  display() {
    this.containerEl.empty();
    this.containerEl.createEl("h2", { text: "\u63D2\u4EF6\u540C\u6B65 Bridge" });
    this.containerEl.createEl("p", {
      cls: "ops-bridge-status",
      text: "\u53EA\u7F13\u5B58\u8BBE\u7F6E\u9875\u7ED3\u6784\uFF0C\u4E0D\u8BFB\u53D6\u8F93\u5165\u503C\uFF0C\u4E5F\u4E0D\u4F1A\u63A8\u65AD\u5199\u5165\u8DEF\u5F84\u3002\u6293\u53D6\u4F1A\u771F\u5B9E\u6E32\u67D3\u76EE\u6807\u63D2\u4EF6\u8BBE\u7F6E\u9875\u3002"
    });
    const candidates = this.bridge.candidates();
    if (!this.bridge.preferences.selectedPluginId || !candidates.some((item) => item.id === this.bridge.preferences.selectedPluginId)) {
      this.bridge.preferences.selectedPluginId = candidates[0]?.id ?? "";
    }
    new import_obsidian2.Setting(this.containerEl).setName("\u76EE\u6807\u63D2\u4EF6").setDesc("\u9ED8\u8BA4\u4E00\u6B21\u53EA\u6293\u53D6\u4E00\u4E2A\u63D2\u4EF6").addDropdown((dropdown) => {
      for (const candidate of candidates) dropdown.addOption(candidate.id, `${candidate.name} (${candidate.id})`);
      dropdown.setValue(this.bridge.preferences.selectedPluginId);
      dropdown.onChange(async (value) => {
        this.bridge.preferences.selectedPluginId = value;
        await this.bridge.savePreferences();
      });
    });
    new import_obsidian2.Setting(this.containerEl).setName("\u6293\u53D6\u6240\u9009\u63D2\u4EF6").setDesc("\u6253\u5F00\u771F\u5B9E\u8BBE\u7F6E\u9875\u5E76\u7F13\u5B58\u5F53\u524D\u53EF\u89C1\u63A7\u4EF6\u7ED3\u6784\uFF1B\u4E0D\u4F1A\u70B9\u51FB\u6309\u94AE\u6216\u66F4\u6539\u63A7\u4EF6\u503C").addButton((button) => button.setButtonText("\u6293\u53D6").setCta().onClick(async () => {
      const pluginId = this.bridge.preferences.selectedPluginId;
      if (!pluginId) return new import_obsidian2.Notice("\u6CA1\u6709\u53EF\u6293\u53D6\u7684\u5DF2\u542F\u7528\u63D2\u4EF6");
      try {
        await this.bridge.capturePlugin(pluginId);
        new import_obsidian2.Notice(`\u5DF2\u7F13\u5B58 ${pluginId} \u7684\u8FD0\u884C\u65F6\u8BBE\u7F6E\u7ED3\u6784`);
      } catch (error) {
        new import_obsidian2.Notice(`\u6293\u53D6\u5931\u8D25\uFF1A${error instanceof Error ? error.message : String(error)}`, 8e3);
      }
    }));
    new import_obsidian2.Setting(this.containerEl).setName("\u6253\u5F00\u771F\u5B9E\u8BBE\u7F6E\u9875").setDesc("\u76F4\u63A5\u8FDB\u5165\u6240\u9009\u63D2\u4EF6\u5728 Obsidian \u4E2D\u6CE8\u518C\u7684\u8BBE\u7F6E\u9875").addButton((button) => button.setButtonText("\u6253\u5F00").onClick(async () => {
      const pluginId = this.bridge.preferences.selectedPluginId;
      if (pluginId) await this.bridge.openPluginSettings(pluginId);
    }));
    new import_obsidian2.Setting(this.containerEl).setName("\u6279\u91CF\u6293\u53D6").setDesc("\u9010\u4E2A\u6E32\u67D3\u6240\u6709\u5DF2\u542F\u7528\u63D2\u4EF6\u7684\u8BBE\u7F6E\u9875\uFF1B\u5931\u8D25\u4E0D\u4F1A\u4E2D\u65AD\u540E\u7EED\u63D2\u4EF6\uFF0C\u53EF\u5728\u63D2\u4EF6\u4E4B\u95F4\u53D6\u6D88").addButton((button) => button.setButtonText("\u67E5\u770B\u98CE\u9669\u5E76\u5F00\u59CB").onClick(() => {
      new BatchCaptureModal(this.app, this.bridge, candidates.map((item) => item.id)).open();
    }));
  }
};
var BatchCaptureModal = class extends import_obsidian2.Modal {
  constructor(app, bridge, pluginIds) {
    super(app);
    this.bridge = bridge;
    this.pluginIds = pluginIds;
  }
  bridge;
  pluginIds;
  cancelled = false;
  started = false;
  progressEl = null;
  onOpen() {
    this.titleEl.setText("\u6279\u91CF\u6293\u53D6\u8FD0\u884C\u65F6\u8BBE\u7F6E\u7ED3\u6784");
    this.contentEl.createEl("p", {
      text: "\u6BCF\u4E2A\u63D2\u4EF6\u7684\u771F\u5B9E\u8BBE\u7F6E\u9875\u90FD\u53EF\u80FD\u6267\u884C\u7F51\u7EDC\u8BF7\u6C42\u3001\u6587\u4EF6\u626B\u63CF\u6216\u6CE8\u518C\u76D1\u542C\u5668\u3002Bridge \u4E0D\u4F1A\u70B9\u51FB\u52A8\u4F5C\u6216\u4FEE\u6539\u63A7\u4EF6\uFF0C\u4F46\u65E0\u6CD5\u6D88\u9664\u63D2\u4EF6\u81EA\u8EAB\u6E32\u67D3\u4EA7\u751F\u7684\u526F\u4F5C\u7528\u3002"
    });
    this.progressEl = this.contentEl.createEl("div", {
      cls: "ops-bridge-progress",
      text: `\u7B49\u5F85\u786E\u8BA4\uFF0C\u5171 ${this.pluginIds.length} \u4E2A\u63D2\u4EF6\u3002`
    });
    const controls = new import_obsidian2.Setting(this.contentEl);
    controls.addButton((button) => button.setButtonText("\u53D6\u6D88").onClick(() => {
      this.cancelled = true;
      if (!this.started) this.close();
    }));
    controls.addButton((button) => button.setButtonText("\u786E\u8BA4\u5E76\u5F00\u59CB").setWarning().onClick(() => void this.start(button)));
  }
  async start(button) {
    if (this.started) return;
    this.started = true;
    button.setDisabled(true);
    const startedAt = (/* @__PURE__ */ new Date()).toISOString();
    const result = await runSequentialBatch(
      this.pluginIds,
      (pluginId) => this.bridge.capturePlugin(pluginId),
      () => this.cancelled,
      (completed, total, entry) => this.updateProgress(completed, total, entry)
    );
    const finishedAt = (/* @__PURE__ */ new Date()).toISOString();
    await writeBatchReport(
      this.app,
      this.bridge.manifest.dir ?? ".obsidian/plugins/obsidian-plugin-sync-bridge",
      { protocolVersion: BRIDGE_PROTOCOL_VERSION, startedAt, finishedAt, cancelled: result.cancelled, entries: result.results }
    );
    const failures = result.results.filter((entry) => entry.status === "failed");
    if (this.progressEl) {
      this.progressEl.setText(
        `${result.cancelled ? "\u5DF2\u53D6\u6D88" : "\u5DF2\u5B8C\u6210"}\uFF1A\u6210\u529F ${result.results.filter((entry) => entry.status === "success").length}\uFF0C\u5931\u8D25 ${failures.length}\u3002` + (failures.length ? `
\u5931\u8D25\u9879\uFF1A${failures.map((entry) => `${entry.pluginId}: ${entry.message}`).join("\uFF1B")}` : "")
      );
    }
    new import_obsidian2.Notice(result.cancelled ? "\u6279\u91CF\u6293\u53D6\u5DF2\u53D6\u6D88" : `\u6279\u91CF\u6293\u53D6\u5B8C\u6210\uFF0C\u5931\u8D25 ${failures.length} \u9879`, 8e3);
  }
  updateProgress(completed, total, entry) {
    this.progressEl?.setText(`${completed}/${total} ${entry.pluginId}\uFF1A${entry.status === "success" ? "\u6210\u529F" : `\u5931\u8D25 - ${entry.message}`}`);
  }
};
