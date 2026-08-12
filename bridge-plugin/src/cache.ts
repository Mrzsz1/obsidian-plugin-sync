import { normalizePath, type App } from "obsidian";
import {
  BRIDGE_CACHE_VERSION,
  BRIDGE_PROTOCOL_VERSION,
  BRIDGE_VERSION,
  cacheFileName,
  fnv1a64,
  type BridgeCacheFile,
  type BridgeFingerprint,
  type BridgeRuntimeStatus,
  type RuntimeSettingsSnapshot,
} from "./protocol.ts";
import type { BridgePluginCandidate } from "./compat.ts";

export type BatchCaptureEntry = {
  pluginId: string;
  status: "success" | "failed" | "cancelled";
  message: string;
};

export type BatchCaptureReport = {
  protocolVersion: number;
  startedAt: string;
  finishedAt: string;
  cancelled: boolean;
  entries: BatchCaptureEntry[];
};

export function bridgeRoot(pluginDir: string): string {
  return normalizePath(pluginDir);
}

export async function writeRuntimeStatus(
  app: App,
  pluginDir: string,
  obsidianVersion: string,
  locale: string,
): Promise<void> {
  const status: BridgeRuntimeStatus = {
    bridgeVersion: BRIDGE_VERSION,
    protocolVersion: BRIDGE_PROTOCOL_VERSION,
    cacheVersion: BRIDGE_CACHE_VERSION,
    obsidianVersion,
    locale,
    vaultName: app.vault.getName(),
    updatedAt: new Date().toISOString(),
  };
  await writeJsonAtomic(app, normalizePath(`${bridgeRoot(pluginDir)}/bridge-status.json`), status);
}

export async function buildFingerprint(
  app: App,
  candidate: BridgePluginCandidate,
  obsidianVersion: string,
  locale: string,
): Promise<BridgeFingerprint> {
  return {
    pluginId: candidate.id,
    pluginVersion: candidate.version,
    pluginMainHash: await fileFingerprint(app, normalizePath(`${candidate.dir}/main.js`)),
    obsidianVersion,
    locale,
    protocolVersion: BRIDGE_PROTOCOL_VERSION,
    configurationHash: await fileFingerprint(app, normalizePath(`${candidate.dir}/data.json`)),
  };
}

export async function writeSnapshot(
  app: App,
  pluginDir: string,
  fingerprint: BridgeFingerprint,
  snapshot: RuntimeSettingsSnapshot,
): Promise<void> {
  const cache: BridgeCacheFile = {
    cacheVersion: BRIDGE_CACHE_VERSION,
    capturedAt: new Date().toISOString(),
    fingerprint,
    snapshot,
  };
  const path = normalizePath(`${bridgeRoot(pluginDir)}/cache/v${BRIDGE_CACHE_VERSION}/${cacheFileName(snapshot.pluginId)}`);
  await writeJsonAtomic(app, path, cache);
}

export async function writeBatchReport(app: App, pluginDir: string, report: BatchCaptureReport): Promise<void> {
  const path = normalizePath(`${bridgeRoot(pluginDir)}/cache/v${BRIDGE_CACHE_VERSION}/batch-latest.json`);
  await writeJsonAtomic(app, path, report);
}

export function currentLocale(): string {
  return localStorage.getItem("language")
    ?? document.documentElement.lang
    ?? "unknown";
}

async function fileFingerprint(app: App, path: string): Promise<string> {
  if (!await app.vault.adapter.exists(path)) return "missing";
  const bytes = new Uint8Array(await app.vault.adapter.readBinary(path));
  return fnv1a64(bytes);
}

async function writeJsonAtomic(app: App, path: string, value: unknown): Promise<void> {
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

async function ensureDirectory(app: App, path: string): Promise<void> {
  const segments = normalizePath(path).split("/").filter(Boolean);
  let current = "";
  for (const segment of segments) {
    current = current ? `${current}/${segment}` : segment;
    if (!await app.vault.adapter.exists(current)) await app.vault.adapter.mkdir(current);
  }
}
