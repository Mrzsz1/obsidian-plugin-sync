import { useState } from "react";
import {
  AlertTriangle,
  Cable,
  Camera,
  Download,
  ExternalLink,
  LoaderCircle,
  Power,
  RefreshCw,
  ShieldCheck,
  Trash2,
  X,
} from "lucide-react";
import type { SettingsBridgeStatus } from "./types";

type SettingsBridgePanelProps = {
  status: SettingsBridgeStatus | null;
  loading: boolean;
  disabled: boolean;
  draftBlocked: boolean;
  onRefresh: () => Promise<void>;
  onInstall: (allowDowngrade: boolean) => Promise<void>;
  onSetEnabled: (enabled: boolean) => Promise<void>;
  onRemove: () => Promise<void>;
  onCapture: () => Promise<void>;
  onOpenSettings: () => Promise<void>;
};

type LocalAction = "refresh" | "capture" | "open" | null;

function installationLabel(status: SettingsBridgeStatus | null) {
  switch (status?.installation) {
    case "ready": return "已就绪";
    case "disabled": return "已安装，未启用";
    case "version-mismatch": return "需要更新";
    case "invalid": return "安装损坏";
    case "missing": return "未安装";
    default: return "正在检测";
  }
}

function installationTone(status: SettingsBridgeStatus | null) {
  switch (status?.installation) {
    case "ready": return "success";
    case "missing": return "neutral";
    case "disabled":
    case "version-mismatch": return "warning";
    default: return status ? "danger" : "neutral";
  }
}

function snapshotLabel(status: SettingsBridgeStatus | null) {
  switch (status?.snapshot) {
    case "fresh": return `可用 · ${status.fieldCount} 项`;
    case "stale": return "已过期，需重新抓取";
    case "invalid": return "快照无效";
    default: return "尚未抓取";
  }
}

function snapshotTone(status: SettingsBridgeStatus | null) {
  switch (status?.snapshot) {
    case "fresh": return "success";
    case "stale": return "warning";
    case "invalid": return "danger";
    default: return "neutral";
  }
}

export function SettingsBridgePanel({
  status,
  loading,
  disabled,
  draftBlocked,
  onRefresh,
  onInstall,
  onSetEnabled,
  onRemove,
  onCapture,
  onOpenSettings,
}: SettingsBridgePanelProps) {
  const [localAction, setLocalAction] = useState<LocalAction>(null);
  const [updateDialogOpen, setUpdateDialogOpen] = useState(false);
  const [updateAcknowledged, setUpdateAcknowledged] = useState(false);
  const [removeDialogOpen, setRemoveDialogOpen] = useState(false);
  const [removeAcknowledged, setRemoveAcknowledged] = useState(false);
  const ready = status?.installation === "ready";
  const blocked = disabled || loading || draftBlocked || Boolean(localAction);

  async function runLocal(action: Exclude<LocalAction, null>, operation: () => Promise<void>) {
    setLocalAction(action);
    try {
      await operation();
    } finally {
      setLocalAction(null);
    }
  }

  async function confirmUpdate() {
    if (!updateAcknowledged) return;
    setUpdateDialogOpen(false);
    setUpdateAcknowledged(false);
    await onInstall(true);
  }

  async function confirmRemove() {
    if (!removeAcknowledged) return;
    setRemoveDialogOpen(false);
    setRemoveAcknowledged(false);
    await onRemove();
  }

  return (
    <section className="manager-section bridge-section">
      <div className="manager-section-heading">
        <div>
          <Cable size={17} />
          <span>Obsidian 真实设置 Bridge</span>
        </div>
        <div className="manager-section-actions">
          <button
            className="ghost-action mini"
            onClick={() => void runLocal("refresh", onRefresh)}
            disabled={blocked}
            title={draftBlocked ? "请先保存或放弃本软件中的配置草稿" : "刷新 Bridge 与运行时快照状态"}
          >
            {localAction === "refresh" ? <LoaderCircle className="spin-icon" size={15} /> : <RefreshCw size={15} />}
            刷新状态
          </button>
          {ready && (
            <>
              <button
                className="ghost-action mini"
                onClick={() => void runLocal("open", onOpenSettings)}
                disabled={blocked}
                title={draftBlocked ? "请先保存或放弃本软件中的配置草稿" : "在 Obsidian 中打开真实设置"}
              >
                {localAction === "open" ? <LoaderCircle className="spin-icon" size={15} /> : <ExternalLink size={15} />}
                真实设置
              </button>
              <button
                className="primary-action mini"
                onClick={() => void runLocal("capture", onCapture)}
                disabled={blocked}
                title={draftBlocked ? "请先保存或放弃本软件中的配置草稿" : "重新抓取运行时设置结构"}
              >
                {localAction === "capture" ? <LoaderCircle className="spin-icon" size={15} /> : <Camera size={15} />}
                {status.snapshot === "fresh" ? "重新抓取" : "抓取结构"}
              </button>
            </>
          )}
        </div>
      </div>

      <div className="bridge-summary">
        <div className="bridge-summary-copy">
          <ShieldCheck size={19} />
          <div>
            <strong>由 Obsidian 运行时提供真实显示结构</strong>
            <span>缓存控件类型、选项与顺序，不缓存输入值；展示信息不能授予写入路径。</span>
          </div>
        </div>
        <dl className="bridge-status-grid">
          <div>
            <dt>Bridge</dt>
            <dd><span className={`bridge-state ${installationTone(status)}`}>{installationLabel(status)}</span></dd>
          </div>
          <div>
            <dt>版本</dt>
            <dd>{status?.installedVersion ? `v${status.installedVersion}` : `内置 v${status?.bundledVersion ?? "0.1.0"}`}</dd>
          </div>
          <div>
            <dt>运行时快照</dt>
            <dd><span className={`bridge-state ${snapshotTone(status)}`}>{snapshotLabel(status)}</span></dd>
          </div>
          <div>
            <dt>抓取时间</dt>
            <dd>{status?.capturedAt ? status.capturedAt.replace("T", " ").replace("Z", "") : "未记录"}</dd>
          </div>
        </dl>
      </div>

      {status?.warnings.length ? (
        <div className="bridge-warning-list">
          <AlertTriangle size={16} />
          <div>{status.warnings.map((warning) => <p key={warning}>{warning}</p>)}</div>
        </div>
      ) : null}

      <div className="bridge-management-actions">
        {status?.installation === "missing" && (
          <button className="primary-action mini" onClick={() => void onInstall(false)} disabled={blocked}>
            <Download size={15} />
            安装并启用 Bridge
          </button>
        )}
        {(status?.installation === "version-mismatch" || status?.installation === "invalid") && (
          <button className="ghost-action mini danger" onClick={() => setUpdateDialogOpen(true)} disabled={blocked}>
            <Download size={15} />
            备份并修复安装
          </button>
        )}
        {status?.installation === "disabled" && (
          <button className="primary-action mini" onClick={() => void onSetEnabled(true)} disabled={blocked}>
            <Power size={15} />
            启用 Bridge
          </button>
        )}
        {status?.enabled && status.installation !== "missing" && status.installation !== "invalid" && (
          <button className="ghost-action mini" onClick={() => void onSetEnabled(false)} disabled={blocked}>
            <Power size={15} />
            禁用 Bridge
          </button>
        )}
        {status && status.installation !== "missing" && (
          <button className="ghost-action mini danger" onClick={() => setRemoveDialogOpen(true)} disabled={blocked}>
            <Trash2 size={15} />
            移除 Bridge
          </button>
        )}
        {draftBlocked && <span className="bridge-draft-blocked">存在未保存配置，Bridge 跳转暂不可用</span>}
      </div>

      {updateDialogOpen && status && (
        <div className="close-overlay">
          <section className="manager-dialog risk-dialog" role="dialog" aria-modal="true" aria-labelledby="bridge-update-title">
            <header>
              <div>
                <p>Bridge 修复与版本确认</p>
                <h2 id="bridge-update-title">覆盖当前 Bridge 文件</h2>
              </div>
              <button className="icon-button" onClick={() => setUpdateDialogOpen(false)} title="关闭"><X size={17} /></button>
            </header>
            <div className="manager-dialog-body">
              <div className="risk-dialog-explanation">
                <AlertTriangle size={20} />
                <span>当前版本 {status.installedVersion ?? "未知"}，桌面端内置版本 {status.bundledVersion}。操作会先完整备份 Bridge，并保留其缓存和偏好；若当前版本更高，这也是一次明确降级确认。</span>
              </div>
              <label className="confirm-check risk-check">
                <input type="checkbox" checked={updateAcknowledged} onChange={(event) => setUpdateAcknowledged(event.target.checked)} />
                我确认关闭 Obsidian 后备份并覆盖 Bridge，必要时允许降级到内置版本。
              </label>
            </div>
            <footer>
              <button className="ghost-action" onClick={() => setUpdateDialogOpen(false)}>取消</button>
              <button className="ghost-action danger" onClick={() => void confirmUpdate()} disabled={!updateAcknowledged}>
                <Download size={16} />
                继续修复
              </button>
            </footer>
          </section>
        </div>
      )}

      {removeDialogOpen && (
        <div className="close-overlay">
          <section className="manager-dialog danger-dialog" role="dialog" aria-modal="true" aria-labelledby="bridge-remove-title">
            <header>
              <div>
                <p>移除配套插件</p>
                <h2 id="bridge-remove-title">移除当前知识库的 Bridge</h2>
              </div>
              <button className="icon-button" onClick={() => setRemoveDialogOpen(false)} title="关闭"><X size={17} /></button>
            </header>
            <div className="manager-dialog-body">
              <label className="confirm-check danger-check">
                <input type="checkbox" checked={removeAcknowledged} onChange={(event) => setRemoveAcknowledged(event.target.checked)} />
                移除 Bridge 插件目录及其运行时缓存；其他插件不会被修改，操作前会创建恢复备份。
              </label>
            </div>
            <footer>
              <button className="ghost-action" onClick={() => setRemoveDialogOpen(false)}>取消</button>
              <button className="ghost-action danger" onClick={() => void confirmRemove()} disabled={!removeAcknowledged}>
                <Trash2 size={16} />
                确认移除
              </button>
            </footer>
          </section>
        </div>
      )}
    </section>
  );
}
