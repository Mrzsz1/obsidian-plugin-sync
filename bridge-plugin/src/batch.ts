export type BatchItemResult = {
  pluginId: string;
  status: "success" | "failed" | "cancelled";
  message: string;
};

export async function runSequentialBatch(
  pluginIds: string[],
  capture: (pluginId: string) => Promise<void>,
  shouldCancel: () => boolean,
  onProgress?: (completed: number, total: number, result: BatchItemResult) => void,
): Promise<{ cancelled: boolean; results: BatchItemResult[] }> {
  const results: BatchItemResult[] = [];
  for (const pluginId of pluginIds) {
    if (shouldCancel()) {
      results.push({ pluginId, status: "cancelled", message: "已在开始此插件前取消" });
      break;
    }
    let result: BatchItemResult;
    try {
      await capture(pluginId);
      result = { pluginId, status: "success", message: "已缓存运行时设置结构" };
    } catch (error) {
      result = {
        pluginId,
        status: "failed",
        message: error instanceof Error ? error.message : String(error),
      };
    }
    results.push(result);
    onProgress?.(results.length, pluginIds.length, result);
  }
  return { cancelled: shouldCancel(), results };
}
