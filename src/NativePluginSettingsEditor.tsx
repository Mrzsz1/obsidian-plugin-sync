import { useEffect, useMemo, useState } from "react";
import {
  AlertTriangle,
  ArrowDown,
  ArrowUp,
  Braces,
  ChevronRight,
  Info,
  LoaderCircle,
  Plus,
  Trash2,
} from "lucide-react";
import type {
  JsonValue,
  PluginSettingField,
  PluginSettingsSchema,
} from "./types";

type NativePluginSettingsEditorProps = {
  schema: PluginSettingsSchema | null;
  value: JsonValue;
  loading: boolean;
  disabled: boolean;
  allowRiskyEdits: boolean;
  onChange: (value: JsonValue) => void;
};

type JsonKind = "object" | "array" | "string" | "number" | "boolean" | "null";

const schemaSourceLabels: Record<PluginSettingsSchema["source"], string> = {
  declarative: "插件定义",
  imperative: "插件代码",
  mixed: "混合推断",
  "data-json": "未识别",
};

const completenessLabels: Record<PluginSettingsSchema["completeness"], string> = {
  complete: "完整",
  partial: "部分",
  fallback: "受限",
};

const supportLabels: Record<PluginSettingField["support"], string> = {
  "safe-writable": "可直接编辑",
  "risk-transform": "需风险确认",
  "dynamic-existing-key": "已有动态键",
  "action-only": "Obsidian 动作",
  "unresolved-runtime": "需运行时采集",
  "unsupported-custom": "自定义界面",
};

const pageLabels: Record<string, string> = {
  general: "General",
  claude: "Claude",
  codex: "Codex",
  opencode: "OpenCode",
  pi: "Pi",
};

function pageLabel(page: string) {
  if (pageLabels[page.toLowerCase()]) return pageLabels[page.toLowerCase()];
  return page
    .replace(/[_-]+/g, " ")
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .replace(/\b\w/g, (letter) => letter.toUpperCase());
}

export function NativePluginSettingsEditor({
  schema,
  value,
  loading,
  disabled,
  allowRiskyEdits,
  onChange,
}: NativePluginSettingsEditorProps) {
  const pages = useMemo(
    () => Array.from(new Set((schema?.groups ?? []).flatMap((group) => group.pagePath.slice(0, 1)))),
    [schema],
  );
  const [selectedPage, setSelectedPage] = useState("");

  useEffect(() => {
    setSelectedPage((current) => (current && pages.includes(current) ? current : pages[0] ?? ""));
  }, [pages.join("\u0000")]);

  if (loading) {
    return (
      <div className="native-settings-loading">
        <LoaderCircle className="spin-icon" size={20} />
        <span>正在分析插件设置</span>
      </div>
    );
  }

  if (!schema) {
    return (
      <div className="native-settings-surface">
        <div className="settings-schema-warning">
          <Info size={16} />
          <span>未能安全识别插件设置。为避免暴露内部参数，未显示完整 data.json。</span>
        </div>
      </div>
    );
  }

  const activePage = selectedPage || pages[0] || "";
  const visibleGroups = schema.groups.filter(
    (group) => group.pagePath.length === 0 || group.pagePath[0] === activePage,
  );
  const fieldCount = schema.coverage.total;
  const bridgeCount = schema.coverage.actionOnly
    + schema.coverage.unresolvedRuntime
    + schema.coverage.unsupportedCustom;

  return (
    <div className="native-settings-surface">
      <div className="settings-schema-summary">
        <span className={`settings-schema-badge ${schema.completeness}`}>
          {schemaSourceLabels[schema.source]} · {completenessLabels[schema.completeness]}
        </span>
        <span>{fieldCount} 项设置</span>
      </div>

      <div className="settings-coverage-summary" aria-label="设置支持情况">
        {schema.coverage.safeWritable > 0 && <span className="safe">直接编辑 {schema.coverage.safeWritable}</span>}
        {schema.coverage.riskTransform > 0 && <span className="risk">风险确认 {schema.coverage.riskTransform}</span>}
        {schema.coverage.dynamicExistingKey > 0 && (
          <span className="dynamic">动态键 {schema.coverage.dynamicExistingKey}</span>
        )}
        {bridgeCount > 0 && <span className="bridge">需 Obsidian {bridgeCount}</span>}
      </div>

      {schema.warnings.length > 0 && (
        <div className="settings-schema-warning" title={schema.warnings.join("\n") }>
          <AlertTriangle size={16} />
          <span>{schema.warnings[0]}</span>
          {schema.warnings.length > 1 && <strong>+{schema.warnings.length - 1}</strong>}
        </div>
      )}

      {pages.length > 1 && (
        <nav className="native-settings-tabs" aria-label="设置页面">
          {pages.map((page) => (
            <button
              key={page}
              className={activePage === page ? "active" : ""}
              onClick={() => setSelectedPage(page)}
            >
              {pageLabel(page)}
            </button>
          ))}
        </nav>
      )}

      <div className="native-settings-groups">
        {visibleGroups.map((group) => (
          <section className="native-settings-group" key={group.id}>
            {group.title && <h4>{group.title}</h4>}
            {group.fields.map((field) => (
              <NativeSettingRow
                key={field.id}
                field={field}
                rootValue={value}
                disabled={disabled}
                allowRiskyEdits={allowRiskyEdits}
                onRootChange={onChange}
              />
            ))}
          </section>
        ))}
      </div>

      {fieldCount === 0 && (
        <div className="native-settings-empty">
          <Braces size={22} />
          <strong>无法安全识别全部设置</strong>
          <span>该插件使用动态或自定义设置界面；程序不会把 data.json 中的内部字段自动当作设置。</span>
        </div>
      )}
    </div>
  );
}

type NativeSettingRowProps = {
  field: PluginSettingField;
  rootValue: JsonValue;
  disabled: boolean;
  allowRiskyEdits: boolean;
  onRootChange: (value: JsonValue) => void;
};

function NativeSettingRow({ field, rootValue, disabled, allowRiskyEdits, onRootChange }: NativeSettingRowProps) {
  const [selectedDynamicPath, setSelectedDynamicPath] = useState("");
  const dynamicPathKey = field.pathOptions.map((option) => option.path).join("\u0000");

  useEffect(() => {
    setSelectedDynamicPath("");
  }, [field.id, field.path, dynamicPathKey]);

  const effectivePath = field.path ?? (selectedDynamicPath || null);
  const hasKnownPath = effectivePath !== null;
  const effectiveReadOnly = disabled || !hasKnownPath || (field.readOnly && !allowRiskyEdits);
  const riskEditable = !disabled && field.readOnly && hasKnownPath && allowRiskyEdits;
  const current = field.path !== null
    ? firstJsonPointerValue(rootValue, field.readPaths.length > 0 ? field.readPaths : [field.path])
    : effectivePath === null ? undefined : getJsonPointer(rootValue, effectivePath);
  const value = current === undefined ? defaultValueForField(field) : current;
  const update = (next: JsonValue) => {
    if (effectivePath === null || effectiveReadOnly) return;
    onRootChange(setJsonPointer(rootValue, effectivePath, next));
  };

  return (
    <div className={`native-setting-row ${effectiveReadOnly ? "read-only" : ""} ${riskEditable ? "risk-editable" : ""}`}>
      <div className="native-setting-copy">
        <div className="native-setting-title-row">
          <strong>{field.name}</strong>
          <span className={`native-setting-support ${field.support}`}>{supportLabels[field.support]}</span>
        </div>
        {field.description && <p>{field.description}</p>}
        {field.warnings.length > 0 && (
          <span className="native-setting-note" title={field.warnings.join("\n") }>
            <AlertTriangle size={13} />
            {field.warnings[0]}
          </span>
        )}
      </div>
      <div className="native-setting-control">
        {field.path === null && field.pathOptions.length > 0 ? (
          <div className="dynamic-setting-control">
            <select
              className="native-select dynamic-path-select"
              value={selectedDynamicPath}
              onChange={(event) => setSelectedDynamicPath(event.target.value)}
              aria-label={`${field.name} 配置键`}
            >
              <option value="">选择已有配置键</option>
              {field.pathOptions.map((option) => (
                <option key={option.path} value={option.path} title={option.detail}>
                  {option.label}
                </option>
              ))}
            </select>
            {selectedDynamicPath ? (
              <SettingControl field={field} value={value} readOnly={effectiveReadOnly} onChange={update} />
            ) : (
              <span className="native-setting-unavailable">选择后可编辑该键</span>
            )}
          </div>
        ) : field.path === null ? (
          <span className="native-setting-unavailable">
            {field.pathOptions.length === 0 && field.warnings.some((warning) => warning.includes("动态"))
              ? "没有可用的已有配置键"
              : "需在 Obsidian 中操作"}
          </span>
        ) : (
          <SettingControl field={field} value={value} readOnly={effectiveReadOnly} onChange={update} />
        )}
      </div>
    </div>
  );
}

type SettingControlProps = {
  field: PluginSettingField;
  value: JsonValue;
  readOnly: boolean;
  onChange: (value: JsonValue) => void;
};

function SettingControl({ field, value, readOnly, onChange }: SettingControlProps) {
  if (readOnly) {
    return <span className="native-setting-unavailable">需在 Obsidian 中操作</span>;
  }

  if (field.control === "unsupported") {
    return <RiskyValueControl value={value} onChange={onChange} />;
  }

  switch (field.control) {
    case "toggle": {
      const checked = Boolean(value);
      return (
        <button
          className={`native-switch ${checked ? "enabled" : ""}`}
          role="switch"
          aria-checked={checked}
          onClick={() => onChange(!checked)}
        >
          <span />
        </button>
      );
    }
    case "textarea":
      return (
        <textarea
          className="native-textarea"
          value={typeof value === "string" ? value : displayValue(value)}
          placeholder={field.placeholder ?? undefined}
          onChange={(event) => onChange(event.target.value)}
        />
      );
    case "dropdown": {
      const selected = serializeOptionValue(value);
      const hasCurrentOption = field.options.some(
        (option) => serializeOptionValue(option.value) === selected,
      );
      return (
        <select
          className="native-select"
          value={selected}
          disabled={field.options.length === 0}
          onChange={(event) => {
            const option = field.options.find(
              (candidate) => serializeOptionValue(candidate.value) === event.target.value,
            );
            if (option) onChange(cloneJson(option.value));
          }}
        >
          {!hasCurrentOption && (
            <option value={selected}>
              {field.options.length === 0 ? `当前值：${displayValue(value) || "（空）"}` : displayValue(value)}
            </option>
          )}
          {field.options.map((option) => (
            <option key={serializeOptionValue(option.value)} value={serializeOptionValue(option.value)}>
              {option.label}
            </option>
          ))}
        </select>
      );
    }
    case "slider": {
      const numeric = typeof value === "number" ? value : Number(value) || field.min || 0;
      if (field.min === null || field.max === null) {
        return (
          <input
            className="native-number-input"
            type="number"
            value={numeric}
            step={field.step ?? undefined}
            onChange={(event) => onChange(Number(event.target.value))}
            title="插件未提供可静态确认的完整滑块范围"
          />
        );
      }
      const step = field.step ?? 1;
      return (
        <div className="native-slider-control">
          <input
            type="range"
            min={field.min}
            max={field.max}
            step={step}
            value={numeric}
            onChange={(event) => onChange(Number(event.target.value))}
          />
          <output>{numeric}</output>
        </div>
      );
    }
    case "number":
      return (
        <input
          className="native-number-input"
          type="number"
          value={typeof value === "number" ? value : Number(value) || 0}
          min={field.min ?? undefined}
          max={field.max ?? undefined}
          step={field.step ?? undefined}
          onChange={(event) => onChange(Number(event.target.value))}
        />
      );
    case "color": {
      const color = typeof value === "string" ? value : "#000000";
      const pickerColor = /^#[0-9a-f]{6}$/i.test(color) ? color : "#000000";
      return (
        <div className="native-color-control">
          <input type="color" value={pickerColor} onChange={(event) => onChange(event.target.value)} />
          <input className="native-text-input compact" value={color} onChange={(event) => onChange(event.target.value)} />
        </div>
      );
    }
    case "nested":
      return <NestedJsonEditor value={value} onChange={onChange} />;
    case "password":
    case "text":
    default:
      return (
        <input
          className="native-text-input"
          type={field.control === "password" ? "password" : "text"}
          value={typeof value === "string" ? value : displayValue(value)}
          placeholder={field.placeholder ?? undefined}
          onChange={(event) => onChange(event.target.value)}
        />
      );
  }
}

function RiskyValueControl({ value, onChange }: Pick<SettingControlProps, "value" | "onChange">) {
  if (value === null || typeof value === "object") {
    return <NestedJsonEditor value={value} onChange={onChange} />;
  }
  if (typeof value === "boolean") {
    return (
      <button
        className={`native-switch ${value ? "enabled" : ""}`}
        role="switch"
        aria-checked={value}
        onClick={() => onChange(!value)}
      >
        <span />
      </button>
    );
  }
  if (typeof value === "number") {
    return (
      <input
        className="native-number-input"
        type="number"
        value={value}
        onChange={(event) => onChange(Number(event.target.value))}
      />
    );
  }
  return <input className="native-text-input" value={value} onChange={(event) => onChange(event.target.value)} />;
}

type NestedJsonEditorProps = {
  value: JsonValue;
  onChange: (value: JsonValue) => void;
  depth?: number;
  onRemove?: () => void;
  onMoveUp?: () => void;
  onMoveDown?: () => void;
};

export function NestedJsonEditor({
  value,
  onChange,
  depth = 0,
  onRemove,
  onMoveUp,
  onMoveDown,
}: NestedJsonEditorProps) {
  const kind = jsonKind(value);
  const [expanded, setExpanded] = useState(depth < 2);
  const objectValue = kind === "object" ? value as Record<string, JsonValue> : null;
  const arrayValue = kind === "array" ? value as JsonValue[] : null;
  const complex = objectValue !== null || arrayValue !== null;

  function renameObjectKey(oldKey: string, newKey: string) {
    if (!objectValue || !newKey || (newKey !== oldKey && Object.hasOwn(objectValue, newKey))) return;
    const next: Record<string, JsonValue> = {};
    for (const [key, child] of Object.entries(objectValue)) {
      next[key === oldKey ? newKey : key] = child;
    }
    onChange(next);
  }

  function addObjectField() {
    if (!objectValue) return;
    let suffix = 1;
    let key = "newField";
    while (Object.hasOwn(objectValue, key)) key = `newField${++suffix}`;
    onChange({ ...objectValue, [key]: "" });
    setExpanded(true);
  }

  return (
    <div className={`native-nested-editor depth-${Math.min(depth, 3)}`}>
      <div className="native-nested-toolbar">
        {complex && (
          <button className="nested-disclosure" onClick={() => setExpanded((current) => !current)} title={expanded ? "收起" : "展开"}>
            <ChevronRight className={expanded ? "expanded" : ""} size={15} />
          </button>
        )}
        <select
          value={kind}
          onChange={(event) => onChange(defaultForKind(event.target.value as JsonKind))}
          aria-label="配置值类型"
        >
          <option value="object">对象</option>
          <option value="array">数组</option>
          <option value="string">文本</option>
          <option value="number">数字</option>
          <option value="boolean">开关</option>
          <option value="null">空值</option>
        </select>
        <span className="nested-summary">{nestedSummary(value)}</span>
        <span className="nested-spacer" />
        {onMoveUp && <IconAction label="上移" onClick={onMoveUp}><ArrowUp size={14} /></IconAction>}
        {onMoveDown && <IconAction label="下移" onClick={onMoveDown}><ArrowDown size={14} /></IconAction>}
        {onRemove && <IconAction label="删除" danger onClick={onRemove}><Trash2 size={14} /></IconAction>}
      </div>

      {objectValue && expanded && (
        <div className="native-nested-children">
          {Object.entries(objectValue).map(([key, child]) => (
            <div className="native-object-entry" key={key}>
              <input
                className="native-key-input"
                value={key}
                onChange={(event) => renameObjectKey(key, event.target.value)}
                aria-label="字段名称"
              />
              <NestedJsonEditor
                value={child}
                depth={depth + 1}
                onChange={(next) => onChange({ ...objectValue, [key]: next })}
                onRemove={() => onChange(Object.fromEntries(Object.entries(objectValue).filter(([entryKey]) => entryKey !== key)))}
              />
            </div>
          ))}
          <button className="native-add-entry" onClick={addObjectField}>
            <Plus size={14} />
            添加字段
          </button>
        </div>
      )}

      {arrayValue && expanded && (
        <div className="native-nested-children">
          {arrayValue.map((child, index) => (
            <div className="native-array-entry" key={index}>
              <span>{index + 1}</span>
              <NestedJsonEditor
                value={child}
                depth={depth + 1}
                onChange={(next) => onChange(arrayValue.map((item, itemIndex) => itemIndex === index ? next : item))}
                onRemove={() => onChange(arrayValue.filter((_, itemIndex) => itemIndex !== index))}
                onMoveUp={index > 0 ? () => onChange(moveArrayItem(arrayValue, index, index - 1)) : undefined}
                onMoveDown={index < arrayValue.length - 1 ? () => onChange(moveArrayItem(arrayValue, index, index + 1)) : undefined}
              />
            </div>
          ))}
          <button className="native-add-entry" onClick={() => onChange([...arrayValue, ""])}>
            <Plus size={14} />
            添加项目
          </button>
        </div>
      )}

      {!complex && (
        <div className="native-nested-primitive">
          {kind === "string" && <input value={value as string} onChange={(event) => onChange(event.target.value)} />}
          {kind === "number" && <input type="number" value={value as number} onChange={(event) => onChange(Number(event.target.value))} />}
          {kind === "boolean" && (
            <button
              className={`native-switch ${value ? "enabled" : ""}`}
              role="switch"
              aria-checked={Boolean(value)}
              onClick={() => onChange(!value)}
            >
              <span />
            </button>
          )}
          {kind === "null" && <span className="native-null-value">null</span>}
        </div>
      )}
    </div>
  );
}

type IconActionProps = {
  label: string;
  danger?: boolean;
  onClick: () => void;
  children: React.ReactNode;
};

function IconAction({ label, danger = false, onClick, children }: IconActionProps) {
  return (
    <button className={`nested-icon-action ${danger ? "danger" : ""}`} onClick={onClick} title={label} aria-label={label}>
      {children}
    </button>
  );
}

function defaultValueForField(field: PluginSettingField): JsonValue {
  if (field.defaultValue !== null) return cloneJson(field.defaultValue);
  switch (field.control) {
    case "toggle": return false;
    case "number": return 0;
    case "slider": return field.min ?? 0;
    case "dropdown": return field.options[0] ? cloneJson(field.options[0].value) : "";
    case "nested": return {};
    default: return "";
  }
}

function getJsonPointer(root: JsonValue, pointer: string): JsonValue | undefined {
  if (pointer === "") return root;
  let current: JsonValue | undefined = root;
  for (const segment of decodePointer(pointer)) {
    if (Array.isArray(current)) {
      const index = Number(segment);
      current = Number.isInteger(index) ? current[index] : undefined;
    } else if (current !== null && typeof current === "object") {
      current = current[segment];
    } else {
      return undefined;
    }
  }
  return current;
}

function firstJsonPointerValue(root: JsonValue, pointers: string[]): JsonValue | undefined {
  for (const pointer of pointers) {
    const value = getJsonPointer(root, pointer);
    if (value !== undefined) return value;
  }
  return undefined;
}

function setJsonPointer(root: JsonValue, pointer: string, nextValue: JsonValue): JsonValue {
  if (pointer === "") return cloneJson(nextValue);
  const segments = decodePointer(pointer);
  const base: JsonValue = root !== null && typeof root === "object"
    ? cloneJson(root)
    : looksLikeArrayIndex(segments[0]) ? [] : {};
  let current = base as JsonValue[] | Record<string, JsonValue>;

  segments.forEach((segment, index) => {
    const last = index === segments.length - 1;
    const nextIsArray = looksLikeArrayIndex(segments[index + 1]);
    if (Array.isArray(current)) {
      const itemIndex = Number(segment);
      if (last) {
        current[itemIndex] = cloneJson(nextValue);
      } else {
        const existing = current[itemIndex];
        if (existing === null || typeof existing !== "object") {
          current[itemIndex] = nextIsArray ? [] : {};
        }
        current = current[itemIndex] as JsonValue[] | Record<string, JsonValue>;
      }
    } else if (last) {
      current[segment] = cloneJson(nextValue);
    } else {
      const existing = current[segment];
      if (existing === null || typeof existing !== "object") {
        current[segment] = nextIsArray ? [] : {};
      }
      current = current[segment] as JsonValue[] | Record<string, JsonValue>;
    }
  });
  return base;
}

function decodePointer(pointer: string) {
  return pointer
    .split("/")
    .slice(1)
    .map((segment) => segment.replace(/~1/g, "/").replace(/~0/g, "~"));
}

function looksLikeArrayIndex(value: string | undefined) {
  return value !== undefined && /^\d+$/.test(value);
}

function jsonKind(value: JsonValue): JsonKind {
  if (value === null) return "null";
  if (Array.isArray(value)) return "array";
  return typeof value as JsonKind;
}

function defaultForKind(kind: JsonKind): JsonValue {
  switch (kind) {
    case "object": return {};
    case "array": return [];
    case "string": return "";
    case "number": return 0;
    case "boolean": return false;
    case "null": return null;
  }
}

function nestedSummary(value: JsonValue) {
  if (Array.isArray(value)) return `${value.length} 项`;
  if (value !== null && typeof value === "object") return `${Object.keys(value).length} 个字段`;
  return "";
}

function moveArrayItem(items: JsonValue[], from: number, to: number) {
  const next = [...items];
  [next[from], next[to]] = [next[to], next[from]];
  return next;
}

function displayValue(value: JsonValue) {
  return typeof value === "string" ? value : JSON.stringify(value);
}

function serializeOptionValue(value: JsonValue) {
  return JSON.stringify(value);
}

function cloneJson<T extends JsonValue>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}
