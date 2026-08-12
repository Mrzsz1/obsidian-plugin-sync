import { useEffect, useRef, useState } from "react";
import {
  Braces,
  Code2,
  Eye,
  EyeOff,
  FileJson,
  LoaderCircle,
  RefreshCw,
  RotateCcw,
  Save,
  ShieldAlert,
  X,
} from "lucide-react";
import { api } from "./api";
import { NestedJsonEditor } from "./NativePluginSettingsEditor";
import type {
  CommandError,
  JsonValue,
  RawConfigDiffEntry,
  RawConfigDiffOperation,
  RawConfigDiffPreview,
  RawPluginConfiguration,
  SyncSummary,
} from "./types";

type RawEditorTab = "structured" | "json";

type RawPluginConfigEditorProps = {
  vaultPath: string;
  pluginId: string;
  pluginName: string;
  disabled: boolean;
  blockedByOtherDrafts: boolean;
  onModeChange: (active: boolean) => void;
  onSave: (proposed: JsonValue, expectedCurrentRevision: string) => Promise<SyncSummary | null>;
};

function commandMessage(error: unknown) {
  const commandError = error as Partial<CommandError>;
  if (commandError?.message) {
    return commandError.path ? `${commandError.message}：${commandError.path}` : commandError.message;
  }
  return error instanceof Error ? error.message : String(error);
}

function cloneJson(value: JsonValue): JsonValue {
  return JSON.parse(JSON.stringify(value)) as JsonValue;
}

function operationLabel(operation: RawConfigDiffOperation) {
  switch (operation) {
    case "add": return "新增";
    case "remove": return "删除";
    default: return "修改";
  }
}

function formatDiffValue(entry: RawConfigDiffEntry, side: "before" | "after", reveal: boolean) {
  const exists = side === "before" ? entry.beforeExists : entry.afterExists;
  if (!exists) return "不存在";
  if (entry.sensitive && !reveal) return "••••••";
  const value = side === "before" ? entry.before : entry.after;
  return JSON.stringify(value);
}

function draftFromSnapshot(snapshot: RawPluginConfiguration): JsonValue | undefined {
  if (snapshot.parseError !== null) return undefined;
  return cloneJson(snapshot.value);
}

export function RawPluginConfigEditor({
  vaultPath,
  pluginId,
  pluginName,
  disabled,
  blockedByOtherDrafts,
  onModeChange,
  onSave,
}: RawPluginConfigEditorProps) {
  const [authorized, setAuthorized] = useState(false);
  const [entryDialogOpen, setEntryDialogOpen] = useState(false);
  const [entryAcknowledged, setEntryAcknowledged] = useState(false);
  const [snapshot, setSnapshot] = useState<RawPluginConfiguration | null>(null);
  const [draft, setDraft] = useState<JsonValue | undefined>(undefined);
  const [rawText, setRawText] = useState("");
  const [parseError, setParseError] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState<RawEditorTab>("structured");
  const [loading, setLoading] = useState(false);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [preview, setPreview] = useState<RawConfigDiffPreview | null>(null);
  const [saveAcknowledged, setSaveAcknowledged] = useState(false);
  const [revealSensitive, setRevealSensitive] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [noticeDanger, setNoticeDanger] = useState(false);
  const requestRef = useRef(0);

  const dirty = Boolean(
    snapshot
      && draft !== undefined
      && (
        !snapshot.exists
        || snapshot.parseError !== null
        || JSON.stringify(draft) !== JSON.stringify(snapshot.value)
      ),
  );
  const hasSensitiveChanges = preview?.entries.some((entry) => entry.sensitive) ?? false;

  useEffect(() => {
    requestRef.current += 1;
    setAuthorized(false);
    setEntryDialogOpen(false);
    setEntryAcknowledged(false);
    setSnapshot(null);
    setDraft(undefined);
    setRawText("");
    setParseError(null);
    setActiveTab("structured");
    setPreview(null);
    setSaveAcknowledged(false);
    setRevealSensitive(false);
    setNotice(null);
  }, [vaultPath, pluginId]);

  useEffect(() => {
    onModeChange(authorized);
  }, [authorized, onModeChange]);

  useEffect(() => () => onModeChange(false), [onModeChange]);

  async function loadConfiguration() {
    const requestId = requestRef.current + 1;
    requestRef.current = requestId;
    setLoading(true);
    setNotice(null);
    try {
      const next = await api.inspectRawManagedPluginConfiguration(vaultPath, pluginId);
      if (requestRef.current !== requestId) return;
      const nextDraft = draftFromSnapshot(next);
      setSnapshot(next);
      setDraft(nextDraft);
      setRawText(next.rawText);
      setParseError(next.parseError);
      setActiveTab(next.parseError ? "json" : "structured");
      setPreview(null);
      setSaveAcknowledged(false);
      setRevealSensitive(false);
    } catch (error) {
      if (requestRef.current !== requestId) return;
      setNoticeDanger(true);
      setNotice(commandMessage(error));
    } finally {
      if (requestRef.current === requestId) setLoading(false);
    }
  }

  async function confirmEntry() {
    if (!entryAcknowledged) return;
    setEntryDialogOpen(false);
    setEntryAcknowledged(false);
    setAuthorized(true);
    await loadConfiguration();
  }

  function updateStructuredDraft(value: JsonValue) {
    setDraft(value);
    setRawText(JSON.stringify(value, null, 2));
    setParseError(null);
    setNotice(null);
  }

  function updateRawText(value: string) {
    setRawText(value);
    setPreview(null);
    try {
      setDraft(JSON.parse(value) as JsonValue);
      setParseError(null);
    } catch (error) {
      setDraft(undefined);
      setParseError(`JSON 格式错误：${error instanceof Error ? error.message : String(error)}`);
    }
  }

  function resetDraft() {
    if (!snapshot) return;
    const nextDraft = draftFromSnapshot(snapshot);
    setDraft(nextDraft);
    setRawText(snapshot.rawText);
    setParseError(snapshot.parseError);
    setActiveTab(snapshot.parseError ? "json" : "structured");
    setPreview(null);
    setNotice(null);
  }

  function closeMode() {
    if (dirty) {
      setNoticeDanger(false);
      setNotice("请先保存或放弃原始配置更改，再退出高级模式");
      return;
    }
    setAuthorized(false);
    setSnapshot(null);
    setDraft(undefined);
    setRawText("");
    setParseError(null);
    setNotice(null);
  }

  async function buildPreview() {
    if (draft === undefined || parseError || !dirty) return;
    setPreviewLoading(true);
    setNotice(null);
    try {
      const next = await api.previewRawManagedPluginConfiguration(vaultPath, pluginId, draft);
      if (next.entries.length === 0) {
        setNoticeDanger(false);
        setNotice("后端重新检查后未发现配置变化");
        await loadConfiguration();
        return;
      }
      setPreview(next);
      setSaveAcknowledged(false);
      setRevealSensitive(false);
    } catch (error) {
      setNoticeDanger(true);
      setNotice(commandMessage(error));
    } finally {
      setPreviewLoading(false);
    }
  }

  async function confirmSave() {
    if (!preview || !saveAcknowledged || draft === undefined) return;
    const proposed = cloneJson(draft);
    setPreview(null);
    setSaveAcknowledged(false);
    setRevealSensitive(false);
    const result = await onSave(proposed, preview.currentRevision);
    if (result && !result.results.some((item) => item.status === "failed")) {
      await loadConfiguration();
    }
  }

  return (
    <section className={`manager-section raw-config-section ${authorized ? "active" : ""}`}>
      <div className="manager-section-heading">
        <div>
          <FileJson size={17} />
          <span>高级原始配置</span>
        </div>
        {!authorized ? (
          <button
            className="ghost-action mini danger"
            onClick={() => setEntryDialogOpen(true)}
            disabled={disabled || blockedByOtherDrafts}
            title={blockedByOtherDrafts ? "请先保存或放弃其他配置草稿" : "编辑完整 data.json"}
          >
            <ShieldAlert size={15} />
            打开高级编辑
          </button>
        ) : (
          <div className="manager-section-actions">
            <button
              className="ghost-action mini"
              onClick={() => void loadConfiguration()}
              disabled={disabled || loading || dirty}
              title={dirty ? "存在未保存更改，不能刷新" : "重新读取 data.json"}
            >
              <RefreshCw className={loading ? "spin-icon" : ""} size={15} />
              刷新
            </button>
            {dirty && (
              <button className="ghost-action mini" onClick={resetDraft} disabled={disabled}>
                <RotateCcw size={15} />
                放弃更改
              </button>
            )}
            <button className="ghost-action mini" onClick={closeMode} disabled={disabled}>
              <X size={15} />
              退出
            </button>
          </div>
        )}
      </div>

      {!authorized ? (
        <div className="raw-config-locked">
          <ShieldAlert size={18} />
          <div>
            <strong>直接编辑完整 data.json</strong>
            <span>适合标准设置和适配器未覆盖的字段，仅影响当前知识库，不会自动同步。</span>
          </div>
        </div>
      ) : (
        <>
          <div className="raw-mode-warning" role="status">
            <ShieldAlert size={17} />
            <span>这里不是插件真实设置页。本软件不会运行插件的校验、转换或副作用逻辑，错误值可能导致插件无法工作。</span>
          </div>

          {notice && (
            <div className={`raw-config-notice ${noticeDanger ? "danger" : "info"}`}>
              {noticeDanger ? <ShieldAlert size={15} /> : <Braces size={15} />}
              <span>{notice}</span>
            </div>
          )}

          {loading ? (
            <div className="raw-config-loading">
              <LoaderCircle className="spin-icon" size={18} />
              正在读取完整配置
            </div>
          ) : snapshot ? (
            <>
              <div className="raw-file-meta">
                <span>{snapshot.exists ? "data.json" : "尚未创建 data.json"}</span>
                <span>{snapshot.byteLength.toLocaleString("zh-CN")} 字节</span>
                {snapshot.parseError && <strong>原文件格式损坏，可用有效 JSON 完整替换</strong>}
              </div>

              <div className="raw-editor-toolbar">
                <div className="raw-editor-tabs" role="tablist" aria-label="原始配置编辑方式">
                  <button
                    className={activeTab === "structured" ? "active" : ""}
                    role="tab"
                    aria-selected={activeTab === "structured"}
                    onClick={() => setActiveTab("structured")}
                    disabled={draft === undefined}
                  >
                    <Braces size={15} />
                    结构化
                  </button>
                  <button
                    className={activeTab === "json" ? "active" : ""}
                    role="tab"
                    aria-selected={activeTab === "json"}
                    onClick={() => setActiveTab("json")}
                  >
                    <Code2 size={15} />
                    原始 JSON
                  </button>
                </div>
                <span className={`raw-draft-state ${parseError ? "danger" : dirty ? "changed" : "clean"}`}>
                  {parseError ? "JSON 无效" : dirty ? "有未保存更改" : "未修改"}
                </span>
              </div>

              {activeTab === "structured" && draft !== undefined ? (
                <fieldset className="raw-structured-editor" disabled={disabled}>
                  <NestedJsonEditor value={draft} onChange={updateStructuredDraft} />
                </fieldset>
              ) : (
                <div className="raw-text-editor">
                  <textarea
                    value={rawText}
                    onChange={(event) => updateRawText(event.target.value)}
                    disabled={disabled}
                    spellCheck={false}
                    aria-label="完整 data.json 内容"
                  />
                  {parseError && (
                    <div className="raw-parse-error">
                      <ShieldAlert size={15} />
                      <span>{parseError}</span>
                    </div>
                  )}
                </div>
              )}

              <div className="raw-editor-footer">
                <span>保存前会由后端重新读取文件、生成完整差异并备份插件目录。</span>
                <button
                  className="primary-action mini"
                  onClick={() => void buildPreview()}
                  disabled={disabled || loading || previewLoading || !dirty || Boolean(parseError) || draft === undefined}
                >
                  {previewLoading ? <LoaderCircle className="spin-icon" size={15} /> : <Save size={15} />}
                  预览并保存
                </button>
              </div>
            </>
          ) : (
            <div className="raw-config-loading">无法读取原始配置，请刷新重试。</div>
          )}
        </>
      )}

      {entryDialogOpen && (
        <div className="close-overlay">
          <section className="manager-dialog risk-dialog" role="dialog" aria-modal="true" aria-labelledby="raw-entry-title">
            <header>
              <div>
                <p>高级功能 · 独立授权</p>
                <h2 id="raw-entry-title">编辑 {pluginName} 的完整 data.json</h2>
              </div>
              <button className="icon-button" onClick={() => setEntryDialogOpen(false)} title="关闭"><X size={17} /></button>
            </header>
            <div className="manager-dialog-body">
              <div className="risk-dialog-explanation">
                <ShieldAlert size={20} />
                <span>此模式会显示普通设置故意隐藏的全部字段，也允许修改任意有效 JSON 值。授权只对当前知识库和当前插件有效。</span>
              </div>
              <label className="confirm-check risk-check">
                <input
                  type="checkbox"
                  checked={entryAcknowledged}
                  onChange={(event) => setEntryAcknowledged(event.target.checked)}
                />
                我理解这不是插件真实设置页，本软件不会执行插件的校验、转换或运行时副作用。
              </label>
            </div>
            <footer>
              <button className="ghost-action" onClick={() => setEntryDialogOpen(false)}>取消</button>
              <button className="ghost-action danger" onClick={() => void confirmEntry()} disabled={!entryAcknowledged}>
                <ShieldAlert size={16} />
                进入原始配置
              </button>
            </footer>
          </section>
        </div>
      )}

      {preview && (
        <div className="close-overlay">
          <section className="manager-dialog raw-diff-dialog" role="dialog" aria-modal="true" aria-labelledby="raw-diff-title">
            <header>
              <div>
                <p>写入前完整差异</p>
                <h2 id="raw-diff-title">确认 {preview.entries.length} 个 JSON 路径变更</h2>
              </div>
              <button className="icon-button" onClick={() => setPreview(null)} title="关闭"><X size={17} /></button>
            </header>
            <div className="manager-dialog-body raw-diff-body">
              <div className="raw-mode-warning">
                <ShieldAlert size={17} />
                <span>写入不会复现插件的验证和运行时效果。确认后仍会要求关闭 Obsidian，并在修改前备份完整插件目录。</span>
              </div>
              {preview.currentParseError && (
                <div className="raw-parse-error">
                  <ShieldAlert size={15} />
                  <span>当前文件无法解析，本次操作将完整替换它：{preview.currentParseError}</span>
                </div>
              )}
              <div className="raw-diff-tools">
                <span>{preview.currentExists ? "当前 data.json 已存在" : "当前没有 data.json"}</span>
                {hasSensitiveChanges && (
                  <button className="ghost-action mini" onClick={() => setRevealSensitive((current) => !current)}>
                    {revealSensitive ? <EyeOff size={15} /> : <Eye size={15} />}
                    {revealSensitive ? "隐藏敏感值" : "显示敏感值"}
                  </button>
                )}
              </div>
              <div className="raw-diff-list">
                {preview.entries.map((entry, index) => (
                  <article className="raw-diff-row" key={`${entry.operation}-${entry.path}-${index}`}>
                    <div className="raw-diff-row-heading">
                      <span className={`raw-operation ${entry.operation}`}>{operationLabel(entry.operation)}</span>
                      <code>{entry.path || "/"}</code>
                      {entry.sensitive && <span className="raw-sensitive-label">敏感字段</span>}
                    </div>
                    <div className="raw-diff-values">
                      <div><span>修改前</span><code>{formatDiffValue(entry, "before", revealSensitive)}</code></div>
                      <div><span>修改后</span><code>{formatDiffValue(entry, "after", revealSensitive)}</code></div>
                    </div>
                  </article>
                ))}
              </div>
              <label className="confirm-check danger-check">
                <input
                  type="checkbox"
                  checked={saveAcknowledged}
                  onChange={(event) => setSaveAcknowledged(event.target.checked)}
                />
                我已检查全部差异，并确认以原始配置方式写入当前插件。
              </label>
            </div>
            <footer>
              <button className="ghost-action" onClick={() => setPreview(null)}>返回编辑</button>
              <button className="ghost-action danger" onClick={() => void confirmSave()} disabled={!saveAcknowledged || disabled}>
                <Save size={16} />
                确认并继续
              </button>
            </footer>
          </section>
        </div>
      )}
    </section>
  );
}
