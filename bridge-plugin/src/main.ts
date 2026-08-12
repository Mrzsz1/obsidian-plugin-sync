import {
  apiVersion,
  ButtonComponent,
  Modal,
  Notice,
  Plugin,
  PluginSettingTab,
  Setting,
  type App,
} from "obsidian";
import { runSequentialBatch, type BatchItemResult } from "./batch.ts";
import {
  buildFingerprint,
  currentLocale,
  writeBatchReport,
  writeRuntimeStatus,
  writeSnapshot,
} from "./cache.ts";
import {
  getPluginCandidates,
  openPluginSettingsTab,
  openSettingsManager,
  renderPluginSettingsTab,
  resolvePluginCandidate,
} from "./compat.ts";
import {
  BRIDGE_PROTOCOL_VERSION,
  BRIDGE_URI_ACTION,
  parseBridgeRequest,
  validateBridgeRequestVault,
  type RuntimeSettingsSnapshot,
} from "./protocol.ts";
import { captureWithSettingInstrumentation } from "./recorder.ts";

type BridgePreferences = {
  selectedPluginId: string;
};

const DEFAULT_PREFERENCES: BridgePreferences = { selectedPluginId: "" };

export default class ObsidianPluginSyncBridge extends Plugin {
  preferences: BridgePreferences = DEFAULT_PREFERENCES;

  async onload(): Promise<void> {
    this.preferences = Object.assign({}, DEFAULT_PREFERENCES, await this.loadData());
    await this.writeStatus();
    this.addSettingTab(new BridgeSettingsTab(this.app, this));
    this.addCommand({
      id: "open-bridge-settings",
      name: "打开插件同步 Bridge 控制台",
      callback: () => void this.openBridgeSettings(),
    });
    this.registerObsidianProtocolHandler(BRIDGE_URI_ACTION, async (data) => {
      try {
        const request = parseBridgeRequest(data);
        validateBridgeRequestVault(request, this.app.vault.getName());
        if (request.operation === "capture") {
          await this.capturePlugin(request.pluginId);
          new Notice(`已缓存 ${request.pluginId} 的运行时设置结构`);
        } else {
          await this.openPluginSettings(request.pluginId);
        }
      } catch (error) {
        new Notice(`Bridge 请求失败：${error instanceof Error ? error.message : String(error)}`, 8_000);
      }
    });
  }

  async savePreferences(): Promise<void> {
    await this.saveData(this.preferences);
  }

  candidates() {
    return getPluginCandidates(this.app);
  }

  async capturePlugin(pluginId: string): Promise<void> {
    const candidate = resolvePluginCandidate(this.app, pluginId);
    await openSettingsManager(this.app);
    const capture = await captureWithSettingInstrumentation(
      Setting.prototype,
      () => renderPluginSettingsTab(this.app, pluginId),
    );
    const snapshot: RuntimeSettingsSnapshot = {
      protocolVersion: BRIDGE_PROTOCOL_VERSION,
      pluginId,
      pluginVersion: candidate.version,
      fields: capture.fields,
      warnings: [
        "运行目标设置页可能触发插件自身的网络、文件扫描或监听器副作用；Bridge 未调用任何设置动作",
        ...capture.warnings,
      ],
    };
    const locale = currentLocale();
    const fingerprint = await buildFingerprint(this.app, candidate, apiVersion, locale);
    await writeSnapshot(this.app, this.manifest.dir ?? ".obsidian/plugins/obsidian-plugin-sync-bridge", fingerprint, snapshot);
    await this.writeStatus();
  }

  async openPluginSettings(pluginId: string): Promise<void> {
    await openPluginSettingsTab(this.app, pluginId);
  }

  async openBridgeSettings(): Promise<void> {
    await openPluginSettingsTab(this.app, this.manifest.id);
  }

  async writeStatus(): Promise<void> {
    await writeRuntimeStatus(
      this.app,
      this.manifest.dir ?? ".obsidian/plugins/obsidian-plugin-sync-bridge",
      apiVersion,
      currentLocale(),
    );
  }
}

class BridgeSettingsTab extends PluginSettingTab {
  constructor(app: App, private readonly bridge: ObsidianPluginSyncBridge) {
    super(app, bridge);
  }

  display(): void {
    this.containerEl.empty();
    this.containerEl.createEl("h2", { text: "插件同步 Bridge" });
    this.containerEl.createEl("p", {
      cls: "ops-bridge-status",
      text: "只缓存设置页结构，不读取输入值，也不会推断写入路径。抓取会真实渲染目标插件设置页。",
    });
    const candidates = this.bridge.candidates();
    if (!this.bridge.preferences.selectedPluginId || !candidates.some((item) => item.id === this.bridge.preferences.selectedPluginId)) {
      this.bridge.preferences.selectedPluginId = candidates[0]?.id ?? "";
    }

    new Setting(this.containerEl)
      .setName("目标插件")
      .setDesc("默认一次只抓取一个插件")
      .addDropdown((dropdown) => {
        for (const candidate of candidates) dropdown.addOption(candidate.id, `${candidate.name} (${candidate.id})`);
        dropdown.setValue(this.bridge.preferences.selectedPluginId);
        dropdown.onChange(async (value) => {
          this.bridge.preferences.selectedPluginId = value;
          await this.bridge.savePreferences();
        });
      });

    new Setting(this.containerEl)
      .setName("抓取所选插件")
      .setDesc("打开真实设置页并缓存当前可见控件结构；不会点击按钮或更改控件值")
      .addButton((button) => button.setButtonText("抓取").setCta().onClick(async () => {
        const pluginId = this.bridge.preferences.selectedPluginId;
        if (!pluginId) return new Notice("没有可抓取的已启用插件");
        try {
          await this.bridge.capturePlugin(pluginId);
          new Notice(`已缓存 ${pluginId} 的运行时设置结构`);
        } catch (error) {
          new Notice(`抓取失败：${error instanceof Error ? error.message : String(error)}`, 8_000);
        }
      }));

    new Setting(this.containerEl)
      .setName("打开真实设置页")
      .setDesc("直接进入所选插件在 Obsidian 中注册的设置页")
      .addButton((button) => button.setButtonText("打开").onClick(async () => {
        const pluginId = this.bridge.preferences.selectedPluginId;
        if (pluginId) await this.bridge.openPluginSettings(pluginId);
      }));

    new Setting(this.containerEl)
      .setName("批量抓取")
      .setDesc("逐个渲染所有已启用插件的设置页；失败不会中断后续插件，可在插件之间取消")
      .addButton((button) => button.setButtonText("查看风险并开始").onClick(() => {
        new BatchCaptureModal(this.app, this.bridge, candidates.map((item) => item.id)).open();
      }));
  }
}

class BatchCaptureModal extends Modal {
  private cancelled = false;
  private started = false;
  private progressEl: HTMLElement | null = null;

  constructor(
    app: App,
    private readonly bridge: ObsidianPluginSyncBridge,
    private readonly pluginIds: string[],
  ) {
    super(app);
  }

  onOpen(): void {
    this.titleEl.setText("批量抓取运行时设置结构");
    this.contentEl.createEl("p", {
      text: "每个插件的真实设置页都可能执行网络请求、文件扫描或注册监听器。Bridge 不会点击动作或修改控件，但无法消除插件自身渲染产生的副作用。",
    });
    this.progressEl = this.contentEl.createEl("div", {
      cls: "ops-bridge-progress",
      text: `等待确认，共 ${this.pluginIds.length} 个插件。`,
    });
    const controls = new Setting(this.contentEl);
    controls.addButton((button) => button.setButtonText("取消").onClick(() => {
      this.cancelled = true;
      if (!this.started) this.close();
    }));
    controls.addButton((button) => button.setButtonText("确认并开始").setWarning().onClick(() => void this.start(button)));
  }

  private async start(button: ButtonComponent): Promise<void> {
    if (this.started) return;
    this.started = true;
    button.setDisabled(true);
    const startedAt = new Date().toISOString();
    const result = await runSequentialBatch(
      this.pluginIds,
      (pluginId) => this.bridge.capturePlugin(pluginId),
      () => this.cancelled,
      (completed, total, entry) => this.updateProgress(completed, total, entry),
    );
    const finishedAt = new Date().toISOString();
    await writeBatchReport(
      this.app,
      this.bridge.manifest.dir ?? ".obsidian/plugins/obsidian-plugin-sync-bridge",
      { protocolVersion: BRIDGE_PROTOCOL_VERSION, startedAt, finishedAt, cancelled: result.cancelled, entries: result.results },
    );
    const failures = result.results.filter((entry) => entry.status === "failed");
    if (this.progressEl) {
      this.progressEl.setText(
        `${result.cancelled ? "已取消" : "已完成"}：成功 ${result.results.filter((entry) => entry.status === "success").length}，失败 ${failures.length}。`
        + (failures.length ? `\n失败项：${failures.map((entry) => `${entry.pluginId}: ${entry.message}`).join("；")}` : ""),
      );
    }
    new Notice(result.cancelled ? "批量抓取已取消" : `批量抓取完成，失败 ${failures.length} 项`, 8_000);
  }

  private updateProgress(completed: number, total: number, entry: BatchItemResult): void {
    this.progressEl?.setText(`${completed}/${total} ${entry.pluginId}：${entry.status === "success" ? "成功" : `失败 - ${entry.message}`}`);
  }
}
