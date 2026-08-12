import assert from "node:assert/strict";
import test from "node:test";
import {
  BRIDGE_PROTOCOL_VERSION,
  cacheFileName,
  fnv1a64,
  isSensitiveLabel,
  isValidPluginId,
  parseBridgeRequest,
  sanitizeStructureText,
  validateBridgeRequestVault,
} from "../src/protocol.ts";

test("accepts only capture and real-settings requests", () => {
  assert.deepEqual(parseBridgeRequest({
    action: "obsidian-plugin-sync-bridge",
    op: "capture",
    plugin: "example-plugin",
    protocol: String(BRIDGE_PROTOCOL_VERSION),
    vault: "Personal Vault",
  }), {
    operation: "capture",
    pluginId: "example-plugin",
    vaultName: "Personal Vault",
  });
  assert.throws(() => parseBridgeRequest({
    op: "patch",
    plugin: "example-plugin",
    protocol: "1",
    vault: "Personal Vault",
  }), /operation/);
  assert.throws(() => parseBridgeRequest({
    op: "capture",
    patch: "{}",
    plugin: "example-plugin",
    protocol: "1",
    vault: "Personal Vault",
  }), /field/);
});

test("validates plugin ids before building cache paths", () => {
  assert.equal(isValidPluginId("plugin.id-2"), true);
  assert.equal(cacheFileName("plugin.id-2"), "plugin.id-2.json");
  for (const invalid of ["", "..", "../escape", "a/b", "a\\b", " bad"]) {
    assert.equal(isValidPluginId(invalid), false);
    assert.throws(() => cacheFileName(invalid));
  }
});

test("rejects requests addressed to another vault", () => {
  const request = parseBridgeRequest({
    action: "obsidian-plugin-sync-bridge",
    op: "capture",
    plugin: "example-plugin",
    protocol: String(BRIDGE_PROTOCOL_VERSION),
    vault: "Personal Vault",
  });
  assert.doesNotThrow(() => validateBridgeRequestVault(request, "Personal Vault"));
  assert.throws(() => validateBridgeRequestVault(request, "Work Vault"), /当前知识库不一致/);
});

test("recognizes secret-like labels without treating monkey as a key", () => {
  assert.equal(isSensitiveLabel("OpenAI API key"), true);
  assert.equal(isSensitiveLabel("Access token"), true);
  assert.equal(isSensitiveLabel("Monkey mode"), false);
});

test("sanitizes structural text and fingerprints bytes deterministically", () => {
  assert.equal(sanitizeStructureText("  Label\u0000 text  "), "Label text");
  assert.equal(sanitizeStructureText("   "), null);
  assert.equal(fnv1a64(new TextEncoder().encode("hello")), "5-a430d84680aabd0b");
});
