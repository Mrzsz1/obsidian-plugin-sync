import { open } from "@tauri-apps/plugin-dialog";
import { AlertTriangle, FolderOpen, HardDrive, ShieldCheck } from "lucide-react";
import type {
  JsonValue,
  PluginAdapterSettingField,
  PluginSettingsAdapterInfo,
} from "./types";

type PluginAdapterSettingsEditorProps = {
  adapter: PluginSettingsAdapterInfo;
  values: Record<string, JsonValue>;
  disabled: boolean;
  onChange: (fieldId: string, value: JsonValue) => void;
};

const portabilityLabels: Record<PluginAdapterSettingField["portability"], string> = {
  portable: "可同步",
  "device-local": "设备本地",
  "vault-local": "知识库本地",
};

export function PluginAdapterSettingsEditor({
  adapter,
  values,
  disabled,
  onChange,
}: PluginAdapterSettingsEditorProps) {
  const compatible = adapter.status === "compatible";

  return (
    <div className="adapter-settings-surface">
      <div className="adapter-settings-summary">
        <span className={`adapter-status-badge ${compatible ? "compatible" : "mismatch"}`}>
          {compatible ? <ShieldCheck size={14} /> : <AlertTriangle size={14} />}
          {adapter.name}
        </span>
        <span>
          {adapter.installedVersion ? `v${adapter.installedVersion.replace(/^v/i, "")}` : "版本未知"}
          {" · "}
          {adapter.versionRequirement}
        </span>
      </div>

      {adapter.warnings.map((warning) => (
        <div className="settings-schema-warning" key={warning}>
          <AlertTriangle size={16} />
          <span>{warning}</span>
        </div>
      ))}

      {compatible && (
        <div className="native-settings-groups">
          {adapter.fields.map((field) => (
            <AdapterSettingRow
              key={field.id}
              field={field}
              value={Object.prototype.hasOwnProperty.call(values, field.id) ? values[field.id] : field.value}
              disabled={disabled || !field.writable}
              onChange={(value) => onChange(field.id, value)}
            />
          ))}
        </div>
      )}
    </div>
  );
}

type AdapterSettingRowProps = {
  field: PluginAdapterSettingField;
  value: JsonValue;
  disabled: boolean;
  onChange: (value: JsonValue) => void;
};

function AdapterSettingRow({ field, value, disabled, onChange }: AdapterSettingRowProps) {
  return (
    <div className={`native-setting-row adapter-setting-row ${disabled ? "read-only" : ""}`}>
      <div className="native-setting-copy">
        <div className="native-setting-title-row">
          <strong>{field.name}</strong>
          <span className={`adapter-portability ${field.portability}`}>
            <HardDrive size={11} />
            {portabilityLabels[field.portability]}
          </span>
        </div>
        {field.description && <p>{field.description}</p>}
        {field.warnings.length > 0 && (
          <span className="native-setting-note" title={field.warnings.join("\n")}>
            <AlertTriangle size={13} />
            {field.warnings[0]}
          </span>
        )}
      </div>
      <div className="native-setting-control">
        {field.writable ? (
          <AdapterControl field={field} value={value} disabled={disabled} onChange={onChange} />
        ) : (
          <span className="native-setting-unavailable">当前设备无法安全定位</span>
        )}
      </div>
    </div>
  );
}

function AdapterControl({ field, value, disabled, onChange }: AdapterSettingRowProps) {
  if (field.control === "toggle") {
    const enabled = value === true;
    return (
      <button
        className={`native-switch ${enabled ? "enabled" : ""}`}
        role="switch"
        aria-checked={enabled}
        disabled={disabled}
        onClick={() => onChange(!enabled)}
      >
        <span />
      </button>
    );
  }

  if (field.control === "dropdown") {
    return (
      <select
        className="native-select"
        value={serializedOptionValue(value)}
        disabled={disabled}
        onChange={(event) => {
          const option = field.options.find(
            (candidate) => serializedOptionValue(candidate.value) === event.target.value,
          );
          if (option) onChange(option.value);
        }}
      >
        {field.options.map((option) => (
          <option key={serializedOptionValue(option.value)} value={serializedOptionValue(option.value)}>
            {option.label}
          </option>
        ))}
      </select>
    );
  }

  if (field.control === "number" || field.control === "slider") {
    return (
      <input
        className="native-number-input"
        type="number"
        value={typeof value === "number" ? value : ""}
        disabled={disabled}
        onChange={(event) => onChange(Number(event.target.value))}
      />
    );
  }

  if (field.control === "textarea") {
    return (
      <textarea
        className="native-textarea"
        value={typeof value === "string" ? value : ""}
        disabled={disabled}
        onChange={(event) => onChange(event.target.value)}
      />
    );
  }

  const textValue = typeof value === "string" ? value : "";
  const canChooseFile = field.id.includes("cli-path");

  async function chooseFile() {
    const selected = await open({
      directory: false,
      multiple: false,
      title: `选择 ${field.name}`,
    });
    if (typeof selected === "string") onChange(selected);
  }

  return (
    <div className="adapter-path-control">
      <input
        className="native-text-input"
        type={field.control === "password" ? "password" : "text"}
        value={textValue}
        disabled={disabled}
        onChange={(event) => onChange(event.target.value)}
      />
      {canChooseFile && (
        <button
          className="icon-button adapter-file-button"
          title="选择文件"
          disabled={disabled}
          onClick={() => void chooseFile()}
        >
          <FolderOpen size={16} />
        </button>
      )}
    </div>
  );
}

function serializedOptionValue(value: JsonValue) {
  return JSON.stringify(value);
}
