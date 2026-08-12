import {
  isSensitiveLabel,
  sanitizeStructureText,
  type RuntimeControl,
  type RuntimeSettingField,
  type RuntimeSettingOption,
} from "./protocol.ts";

type Callable = (this: unknown, ...args: unknown[]) => unknown;

type SettingLike = {
  settingEl?: HTMLElement;
  nameEl?: HTMLElement;
  descEl?: HTMLElement;
  controlEl?: HTMLElement;
};

type MutableRuntimeRow = {
  setting: SettingLike;
  order: number;
  control: RuntimeControl;
  options: RuntimeSettingOption[];
  placeholder: string | null;
  min: number | null;
  max: number | null;
  step: number | null;
  disabled: boolean;
  heading: boolean;
  hasValueControl: boolean;
  hasAction: boolean;
};

type PrototypePatch = {
  name: string;
  descriptor: PropertyDescriptor;
};

export type RuntimeCaptureResult = {
  fields: RuntimeSettingField[];
  warnings: string[];
};

const VALUE_COMPONENTS: Array<[string, RuntimeControl]> = [
  ["addToggle", "toggle"],
  ["addText", "text"],
  ["addSearch", "text"],
  ["addTextArea", "textarea"],
  ["addDropdown", "dropdown"],
  ["addColorPicker", "color"],
  ["addSlider", "slider"],
];

const ACTION_COMPONENTS = ["addButton", "addExtraButton"];

class RuntimeRecorder {
  private readonly rows = new Map<object, MutableRuntimeRow>();
  private readonly orderedRows: MutableRuntimeRow[] = [];

  touch(setting: unknown): MutableRuntimeRow {
    const key = setting as object;
    const existing = this.rows.get(key);
    if (existing) return existing;
    const row: MutableRuntimeRow = {
      setting: setting as SettingLike,
      order: this.orderedRows.length,
      control: "unsupported",
      options: [],
      placeholder: null,
      min: null,
      max: null,
      step: null,
      disabled: false,
      heading: false,
      hasValueControl: false,
      hasAction: false,
    };
    this.rows.set(key, row);
    this.orderedRows.push(row);
    return row;
  }

  setHeading(setting: unknown): void {
    const row = this.touch(setting);
    row.heading = true;
    row.control = "heading";
  }

  setDisabled(setting: unknown, disabled: boolean): void {
    this.touch(setting).disabled = disabled;
  }

  captureValueComponent(setting: unknown, control: RuntimeControl, component: unknown): void {
    const row = this.touch(setting);
    if (!row.hasValueControl) row.control = control;
    row.hasValueControl = true;
    const record = component as Record<string, unknown>;
    const input = record.inputEl as HTMLInputElement | HTMLTextAreaElement | undefined;
    const select = record.selectEl as HTMLSelectElement | undefined;
    const slider = record.sliderEl as HTMLInputElement | undefined;
    const baseDisabled = record.disabled === true;

    if (input) {
      row.placeholder = sanitizeStructureText(input.placeholder, 200);
      row.disabled ||= baseDisabled || input.disabled;
      if (input.type === "password") row.control = "password";
      if (input.type === "number") row.control = "number";
    } else if (select) {
      row.disabled ||= baseDisabled || select.disabled;
      row.options = Array.from(select.options)
        .slice(0, 500)
        .map((option) => ({
          value: sanitizeStructureText(option.value, 500) ?? "",
          label: sanitizeStructureText(option.textContent, 500) ?? option.value,
        }));
    } else if (slider) {
      row.disabled ||= baseDisabled || slider.disabled;
      row.min = finiteNumber(slider.min);
      row.max = finiteNumber(slider.max);
      row.step = slider.step === "any" ? null : finiteNumber(slider.step);
    } else {
      row.disabled ||= baseDisabled;
    }
  }

  captureActionComponent(setting: unknown, component: unknown): void {
    const row = this.touch(setting);
    row.hasAction = true;
    const record = component as Record<string, unknown>;
    const button = record.buttonEl as HTMLButtonElement | undefined;
    row.disabled ||= record.disabled === true || button?.disabled === true;
  }

  finish(container: HTMLElement, pagePath: string[]): RuntimeCaptureResult {
    const fields: RuntimeSettingField[] = [];
    const warnings: string[] = [];
    const standardRows = new Set<HTMLElement>();
    let groupTitle: string | null = null;

    for (const row of this.orderedRows) {
      const element = row.setting.settingEl;
      if (!element || !container.contains(element)) continue;
      standardRows.add(element);
      const name = sanitizeStructureText(row.setting.nameEl?.textContent, 500)
        ?? `未命名设置 ${fields.length + 1}`;
      const sensitive = isSensitiveLabel(name);
      const heading = row.heading || element.classList.contains("setting-item-heading");
      const actionOnly = row.hasAction && !row.hasValueControl;
      const control = sensitive && (row.control === "text" || row.control === "textarea")
        ? "password"
        : heading
          ? "heading"
          : row.control;
      fields.push({
        pagePath: [...pagePath],
        groupTitle,
        order: fields.length,
        name,
        description: sensitive ? null : sanitizeStructureText(row.setting.descEl?.textContent, 1_000),
        control,
        options: sensitive ? [] : row.options,
        placeholder: sensitive ? null : row.placeholder,
        min: row.min,
        max: row.max,
        step: row.step,
        disabled: row.disabled || element.getAttribute("aria-disabled") === "true",
        visible: isElementVisible(element),
        action: actionOnly,
        confidence: "exact",
      });
      if (heading) groupTitle = name;
    }

    const fallback = captureCustomControls(container, standardRows, pagePath, fields.length);
    if (fallback.length > 0) {
      warnings.push(`发现 ${fallback.length} 个自定义 DOM 控件；仅缓存低置信度结构，不推断写入路径`);
      fields.push(...fallback);
    }
    return { fields, warnings };
  }
}

export async function captureWithSettingInstrumentation(
  settingPrototype: object,
  render: () => Promise<HTMLElement>,
  pagePath: string[] = [],
): Promise<RuntimeCaptureResult> {
  const recorder = new RuntimeRecorder();
  const patches: PrototypePatch[] = [];
  const prototype = settingPrototype as Record<string, unknown>;

  patchAfter(prototype, "setName", patches, (setting) => recorder.touch(setting));
  patchAfter(prototype, "setDesc", patches, (setting) => recorder.touch(setting));
  patchAfter(prototype, "setHeading", patches, (setting) => recorder.setHeading(setting));
  patchAfter(prototype, "setDisabled", patches, (setting, args) => {
    recorder.setDisabled(setting, args[0] === true);
  });
  for (const [method, control] of VALUE_COMPONENTS) {
    patchComponent(prototype, method, patches, (setting, component) => {
      recorder.captureValueComponent(setting, control, component);
    });
  }
  for (const method of ACTION_COMPONENTS) {
    patchComponent(prototype, method, patches, (setting, component) => {
      recorder.captureActionComponent(setting, component);
    });
  }

  try {
    const container = await render();
    return recorder.finish(container, pagePath);
  } finally {
    for (const patch of patches.reverse()) {
      Object.defineProperty(prototype, patch.name, patch.descriptor);
    }
  }
}

function patchAfter(
  prototype: Record<string, unknown>,
  name: string,
  patches: PrototypePatch[],
  after: (setting: unknown, args: unknown[]) => void,
): void {
  const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
  if (!descriptor || typeof descriptor.value !== "function") return;
  const original = descriptor.value as Callable;
  patches.push({ name, descriptor });
  Object.defineProperty(prototype, name, {
    ...descriptor,
    value: function patchedMethod(this: unknown, ...args: unknown[]) {
      const result = original.apply(this, args);
      after(this, args);
      return result;
    },
  });
}

function patchComponent(
  prototype: Record<string, unknown>,
  name: string,
  patches: PrototypePatch[],
  capture: (setting: unknown, component: unknown) => void,
): void {
  const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
  if (!descriptor || typeof descriptor.value !== "function") return;
  const original = descriptor.value as Callable;
  patches.push({ name, descriptor });
  Object.defineProperty(prototype, name, {
    ...descriptor,
    value: function patchedComponentMethod(this: unknown, ...args: unknown[]) {
      const callback = args[0];
      if (typeof callback !== "function") return original.apply(this, args);
      const wrapped = (component: unknown) => {
        try {
          return (callback as (value: unknown) => unknown)(component);
        } finally {
          capture(this, component);
        }
      };
      return original.apply(this, [wrapped, ...args.slice(1)]);
    },
  });
}

function captureCustomControls(
  container: HTMLElement,
  standardRows: Set<HTMLElement>,
  pagePath: string[],
  startOrder: number,
): RuntimeSettingField[] {
  const fields: RuntimeSettingField[] = [];
  const controls = container.querySelectorAll<HTMLElement>("input, select, textarea, button");
  for (const control of Array.from(controls)) {
    const standard = Array.from(standardRows).some((row) => row.contains(control));
    if (standard) continue;
    const row = control.closest<HTMLElement>(".setting-item") ?? control.parentElement;
    const label = sanitizeStructureText(
      control.getAttribute("aria-label")
        ?? row?.querySelector<HTMLElement>(".setting-item-name, label")?.textContent
        ?? control.getAttribute("placeholder"),
      500,
    ) ?? `自定义控件 ${fields.length + 1}`;
    const sensitive = isSensitiveLabel(label) || control.getAttribute("type") === "password";
    const { control: kind, action } = domControlKind(control, sensitive);
    const tagName = control.tagName.toLowerCase();
    const select = tagName === "select" ? control as HTMLSelectElement : null;
    const input = tagName === "input" ? control as HTMLInputElement : null;
    fields.push({
      pagePath: [...pagePath],
      groupTitle: nearestHeading(control, container),
      order: startOrder + fields.length,
      name: label,
      description: null,
      control: kind,
      options: sensitive || !select
        ? []
        : Array.from(select.options).slice(0, 500).map((option) => ({
            value: sanitizeStructureText(option.value, 500) ?? "",
            label: sanitizeStructureText(option.textContent, 500) ?? option.value,
          })),
      placeholder: sensitive ? null : sanitizeStructureText(control.getAttribute("placeholder"), 200),
      min: input?.type === "range" ? finiteNumber(input.min) : null,
      max: input?.type === "range" ? finiteNumber(input.max) : null,
      step: input?.type === "range" && input.step !== "any"
        ? finiteNumber(input.step)
        : null,
      disabled: "disabled" in control && Boolean((control as HTMLInputElement).disabled),
      visible: isElementVisible(control),
      action,
      confidence: "fallback",
    });
  }
  return fields;
}

function domControlKind(control: HTMLElement, sensitive: boolean): { control: RuntimeControl; action: boolean } {
  const tagName = control.tagName.toLowerCase();
  if (tagName === "button") return { control: "unsupported", action: true };
  if (tagName === "textarea") return { control: sensitive ? "password" : "textarea", action: false };
  if (tagName === "select") return { control: "dropdown", action: false };
  if (tagName === "input") {
    const input = control as HTMLInputElement;
    if (sensitive || input.type === "password") return { control: "password", action: false };
    if (input.type === "checkbox") return { control: "toggle", action: false };
    if (input.type === "range") return { control: "slider", action: false };
    if (input.type === "number") return { control: "number", action: false };
    if (input.type === "color") return { control: "color", action: false };
    return { control: "text", action: false };
  }
  return { control: "unsupported", action: false };
}

function nearestHeading(control: HTMLElement, container: HTMLElement): string | null {
  let heading: string | null = null;
  const following = control.ownerDocument.defaultView?.Node.DOCUMENT_POSITION_FOLLOWING ?? 4;
  for (const candidate of Array.from(container.querySelectorAll<HTMLElement>("h1, h2, h3, h4, .setting-item-heading"))) {
    if (candidate.compareDocumentPosition(control) & following) {
      heading = sanitizeStructureText(candidate.textContent, 500);
    }
  }
  return heading;
}

function finiteNumber(value: string | number | null | undefined): number | null {
  if (value === null || value === undefined || value === "") return null;
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : null;
}

function isElementVisible(element: HTMLElement): boolean {
  if (element.hidden || element.getAttribute("aria-hidden") === "true") return false;
  if (element.classList.contains("is-hidden") || element.style.display === "none") return false;
  if (typeof window !== "undefined" && element.isConnected) {
    const style = window.getComputedStyle(element);
    if (style.display === "none" || style.visibility === "hidden") return false;
  }
  return true;
}
