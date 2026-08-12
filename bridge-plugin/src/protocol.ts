export const BRIDGE_PLUGIN_ID = "obsidian-plugin-sync-bridge";
export const BRIDGE_VERSION = "0.1.0";
export const BRIDGE_PROTOCOL_VERSION = 1;
export const BRIDGE_CACHE_VERSION = 1;
export const BRIDGE_URI_ACTION = BRIDGE_PLUGIN_ID;

export type JsonValue = null | boolean | number | string | JsonValue[] | { [key: string]: JsonValue };

export type RuntimeControl =
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

export type RuntimeConfidence = "exact" | "inferred" | "fallback";

export type RuntimeSettingOption = {
  value: JsonValue;
  label: string;
};

export type RuntimeSettingField = {
  pagePath: string[];
  groupTitle: string | null;
  order: number;
  name: string;
  description: string | null;
  control: RuntimeControl;
  options: RuntimeSettingOption[];
  placeholder: string | null;
  min: number | null;
  max: number | null;
  step: number | null;
  disabled: boolean;
  visible: boolean;
  action: boolean;
  confidence: RuntimeConfidence;
};

export type RuntimeSettingsSnapshot = {
  protocolVersion: number;
  pluginId: string;
  pluginVersion: string | null;
  fields: RuntimeSettingField[];
  warnings: string[];
};

export type BridgeFingerprint = {
  pluginId: string;
  pluginVersion: string | null;
  pluginMainHash: string;
  obsidianVersion: string;
  locale: string;
  protocolVersion: number;
  configurationHash: string;
};

export type BridgeCacheFile = {
  cacheVersion: number;
  capturedAt: string;
  fingerprint: BridgeFingerprint;
  snapshot: RuntimeSettingsSnapshot;
};

export type BridgeRuntimeStatus = {
  bridgeVersion: string;
  protocolVersion: number;
  cacheVersion: number;
  obsidianVersion: string;
  locale: string;
  vaultName: string;
  updatedAt: string;
};

export type BridgeRequest = {
  operation: "capture" | "open-settings";
  vaultName: string;
  pluginId: string;
};

const PLUGIN_ID_PATTERN = /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/;
const REQUEST_KEYS = new Set(["action", "op", "plugin", "protocol", "vault"]);
const SECRET_PATTERN = /(?:^|[\s._-])(api[\s._-]*key|access[\s._-]*token|refresh[\s._-]*token|token|secret|password|passphrase|credential|authorization)(?:$|[\s._-])/i;

export function isValidPluginId(value: string): boolean {
  return PLUGIN_ID_PATTERN.test(value) && value !== "." && value !== "..";
}

export function isSensitiveLabel(value: string): boolean {
  return SECRET_PATTERN.test(` ${value.trim()} `);
}

export function sanitizeStructureText(value: unknown, maxLength = 500): string | null {
  if (typeof value !== "string") return null;
  const normalized = value.replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/g, "").trim();
  if (!normalized) return null;
  return normalized.slice(0, maxLength);
}

export function fnv1a64(bytes: Uint8Array): string {
  let hash = 0xcbf29ce484222325n;
  for (const byte of bytes) {
    hash ^= BigInt(byte);
    hash = BigInt.asUintN(64, hash * 0x100000001b3n);
  }
  return `${bytes.length}-${hash.toString(16).padStart(16, "0")}`;
}

export function utf8Fingerprint(value: string): string {
  return fnv1a64(new TextEncoder().encode(value));
}

export function parseBridgeRequest(data: Record<string, string>): BridgeRequest {
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

export function validateBridgeRequestVault(request: BridgeRequest, currentVaultName: string): void {
  const normalizedCurrentVault = sanitizeStructureText(currentVaultName, 255);
  if (!normalizedCurrentVault || request.vaultName !== normalizedCurrentVault) {
    throw new Error("URI 指定的知识库与当前知识库不一致");
  }
}

export function cacheFileName(pluginId: string): string {
  if (!isValidPluginId(pluginId)) throw new Error("invalid plugin id");
  return `${pluginId}.json`;
}
