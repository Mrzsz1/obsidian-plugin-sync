import assert from "node:assert/strict";
import test from "node:test";
import { captureWithSettingInstrumentation } from "../src/recorder.ts";

type FakeElement = {
  textContent: string;
  hidden: boolean;
  isConnected: boolean;
  style: { display: string };
  classList: { contains: () => boolean };
  getAttribute: () => string | null;
  contains: () => boolean;
  querySelectorAll: () => unknown[];
};

function fakeElement(textContent = ""): FakeElement {
  return {
    textContent,
    hidden: false,
    isConnected: false,
    style: { display: "" },
    classList: { contains: () => false },
    getAttribute: () => null,
    contains: () => true,
    querySelectorAll: () => [],
  };
}

class FakeSetting {
  settingEl = fakeElement();
  nameEl = fakeElement();
  descEl = fakeElement();

  setName(name: string) { this.nameEl.textContent = name; return this; }
  setDesc(description: string) { this.descEl.textContent = description; return this; }
  setHeading() { return this; }
  setDisabled(_disabled: boolean) { return this; }
  addText(callback: (component: unknown) => void) {
    callback({ inputEl: { placeholder: "Enter value", disabled: false, type: "text" }, disabled: false });
    return this;
  }
  addButton(callback: (component: unknown) => void) {
    callback({
      buttonEl: { disabled: false },
      disabled: false,
      onClick(_handler: () => void) { return this; },
    });
    return this;
  }
  addDropdown(callback: (component: unknown) => void) {
    const options: Array<{ value: string; textContent: string }> = [];
    callback({
      disabled: false,
      selectEl: { disabled: false, options },
      addOption(value: string, label: string) { options.push({ value, textContent: label }); return this; },
    });
    return this;
  }
  addSlider(callback: (component: unknown) => void) {
    callback({ disabled: false, sliderEl: { disabled: false, min: "1", max: "60", step: "1" } });
    return this;
  }
}

test("records structure without input values and keeps auxiliary buttons secondary", async () => {
  const container = fakeElement() as unknown as HTMLElement;
  const result = await captureWithSettingInstrumentation(FakeSetting.prototype, async () => {
    new FakeSetting()
      .setName("Model")
      .setDesc("Default model")
      .addText(() => undefined)
      .addButton(() => undefined);
    return container;
  });

  assert.equal(result.fields.length, 1);
  assert.equal(result.fields[0].control, "text");
  assert.equal(result.fields[0].action, false);
  assert.equal(Object.hasOwn(result.fields[0], "value"), false);
});

test("always restores instrumentation after target render throws", async () => {
  const original = FakeSetting.prototype.setName;
  await assert.rejects(
    captureWithSettingInstrumentation(FakeSetting.prototype, async () => {
      new FakeSetting().setName("Before failure");
      throw new Error("plugin failed");
    }),
    /plugin failed/,
  );
  assert.equal(FakeSetting.prototype.setName, original);
});

test("observes button registration without invoking actions and restores after success", async () => {
  const container = fakeElement() as unknown as HTMLElement;
  const original = FakeSetting.prototype.addButton;
  let actionCalls = 0;
  await captureWithSettingInstrumentation(FakeSetting.prototype, async () => {
    new FakeSetting().setName("Dangerous action").addButton((component) => {
      const button = component as { onClick: (handler: () => void) => unknown };
      button.onClick(() => {
        actionCalls += 1;
      });
    });
    return container;
  });

  assert.equal(actionCalls, 0);
  assert.equal(FakeSetting.prototype.addButton, original);
});

test("captures runtime options, limits, visibility, and secret structure without values", async () => {
  const container = fakeElement() as unknown as HTMLElement;
  const result = await captureWithSettingInstrumentation(FakeSetting.prototype, async () => {
    new FakeSetting().setName("Runtime model").addDropdown((component) => {
      const dropdown = component as { addOption: (value: string, label: string) => unknown };
      dropdown.addOption("fast", "Fast");
      dropdown.addOption("accurate", "Accurate");
    });
    new FakeSetting().setName("Timeout").addSlider(() => undefined);
    const conditional = new FakeSetting().setName("Conditional option").addText(() => undefined);
    conditional.settingEl.hidden = true;
    const secret = new FakeSetting().setName("OpenAI API key").setDesc("Current key should not be cached");
    secret.addText((component) => {
      const text = component as { inputEl: Record<string, unknown> };
      text.inputEl.type = "password";
      text.inputEl.placeholder = "sk-current-secret";
      text.inputEl.value = "sk-current-secret";
    });
    return container;
  });

  assert.deepEqual(result.fields[0].options, [
    { value: "fast", label: "Fast" },
    { value: "accurate", label: "Accurate" },
  ]);
  assert.deepEqual(
    { min: result.fields[1].min, max: result.fields[1].max, step: result.fields[1].step },
    { min: 1, max: 60, step: 1 },
  );
  assert.equal(result.fields[2].visible, false);
  assert.equal(result.fields[3].control, "password");
  assert.equal(result.fields[3].description, null);
  assert.equal(result.fields[3].placeholder, null);
  assert.equal(JSON.stringify(result.fields).includes("sk-current-secret"), false);
});
