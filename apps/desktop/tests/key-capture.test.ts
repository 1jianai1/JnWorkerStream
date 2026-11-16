import { describe, expect, it } from "vitest";
import {
  comboFromEvent,
  formatModifierMask,
  formatUsage,
  modifierMask,
  modifierNames,
  supportedKeyCount,
} from "../src/lib/key-capture";

function keydown(init: KeyboardEventInit): KeyboardEvent {
  return new KeyboardEvent("keydown", init);
}

describe("key capture", () => {
  it("captures Ctrl+C with the documented modifier bit and usage", () => {
    const combo = comboFromEvent(keydown({ code: "KeyC", key: "c", ctrlKey: true }));
    expect(combo).not.toBeNull();
    expect(combo?.label).toBe("Ctrl+C");
    expect(combo?.usage).toBe(0x06);
    expect(combo?.modifier).toBe(0x01);
    expect(formatUsage(combo?.usage ?? 0)).toBe("0x06");
    expect(formatModifierMask(combo?.modifier ?? 0)).toBe("0x01");
  });

  it("captures multi-modifier combinations in a stable order", () => {
    const combo = comboFromEvent(keydown({ code: "KeyS", key: "S", shiftKey: true, altKey: true }));
    expect(combo?.label).toBe("Shift+Alt+S");
    expect(combo?.modifier).toBe(0x06);
    expect(combo?.usage).toBe(0x16);
  });

  it("ignores modifier-only keys and auto-repeat", () => {
    expect(comboFromEvent(keydown({ code: "ControlLeft", key: "Control", ctrlKey: true }))).toBeNull();
    expect(comboFromEvent(keydown({ code: "KeyA", key: "a", repeat: true }))).toBeNull();
  });

  it("returns null for unsupported keys", () => {
    expect(comboFromEvent(keydown({ code: "IntlRo", key: "ろ" }))).toBeNull();
  });

  it("exposes stable modifier helpers and a broad key map", () => {
    expect(modifierMask({ ctrlKey: true, shiftKey: true, altKey: false, metaKey: false })).toBe(0x03);
    expect(modifierNames(0x0f)).toEqual(["Ctrl", "Shift", "Alt", "Win"]);
    expect(supportedKeyCount()).toBeGreaterThan(80);
  });
});
