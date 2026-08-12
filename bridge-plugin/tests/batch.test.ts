import assert from "node:assert/strict";
import test from "node:test";
import { runSequentialBatch } from "../src/batch.ts";

test("batch capture is sequential and continues after failures", async () => {
  const active: string[] = [];
  const order: string[] = [];
  const result = await runSequentialBatch(["one", "two", "three"], async (pluginId) => {
    assert.equal(active.length, 0);
    active.push(pluginId);
    order.push(pluginId);
    active.pop();
    if (pluginId === "two") throw new Error("broken settings tab");
  }, () => false);

  assert.deepEqual(order, ["one", "two", "three"]);
  assert.deepEqual(result.results.map((entry) => entry.status), ["success", "failed", "success"]);
  assert.equal(result.cancelled, false);
});

test("batch cancellation is observed only between plugins", async () => {
  let cancelled = false;
  const result = await runSequentialBatch(["one", "two", "three"], async (pluginId) => {
    if (pluginId === "one") cancelled = true;
  }, () => cancelled);

  assert.deepEqual(result.results.map((entry) => [entry.pluginId, entry.status]), [
    ["one", "success"],
    ["two", "cancelled"],
  ]);
  assert.equal(result.cancelled, true);
});
