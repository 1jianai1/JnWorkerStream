import { describe, expect, it } from "vitest";
import type { KeyConfig, MacroStep } from "../src/lib/types";
import {
  isConfigured,
  macroByteEstimate,
  macroDuration,
  stageVisual,
  validateKeyConfig,
} from "../src/lib/keypad";

function config(overrides: Partial<KeyConfig> = {}): KeyConfig {
  return {
    route: "devicePassthrough",
    report: false,
    disabled: false,
    usage: "",
    modifier: "none",
    macro: [],
    action: { kind: "launch", target: "", args: "", shell: "powershell", timeoutMs: 5000 },
    trigger: "tap",
    ...overrides,
  };
}

function keysStep(id: string, keys: string): MacroStep {
  return { id, kind: "keys", keys, holdMs: 20, delayMs: 30 };
}

describe("keypad model", () => {
  it("detects configured keys per route", () => {
    expect(isConfigured(config())).toBe(false);
    expect(isConfigured(config({ usage: "0x06" }))).toBe(true);
    expect(isConfigured(config({ route: "deviceMacro", macro: [keysStep("a", "Ctrl+C")] }))).toBe(true);
    expect(isConfigured(config({ route: "host", action: { kind: "launch", target: "notepad.exe", args: "", shell: "powershell", timeoutMs: 5000 } }))).toBe(true);
  });

  it("derives the write target from the route", () => {
    expect(validateKeyConfig(config()).target).toBe("both");
    expect(validateKeyConfig(config({ route: "deviceMacro" })).target).toBe("both");
    expect(validateKeyConfig(config({ route: "host" })).target).toBe("host");
  });

  it("rejects host routing that still carries device macro steps", () => {
    const result = validateKeyConfig(config({ route: "host", macro: [keysStep("a", "A")] }));
    expect(result.ok).toBe(false);
    expect(result.code).toBe("conflict");
  });

  it("requires a host action target", () => {
    const result = validateKeyConfig(config({ route: "host" }));
    expect(result.ok).toBe(false);
    expect(result.code).toBe("targetRequired");
  });

  it("estimates macro bytes and flags the 200 B ceiling", () => {
    expect(macroByteEstimate([])).toBe(4);
    expect(macroByteEstimate([keysStep("a", "Ctrl+C")])).toBe(13);
    const tooBig = Array.from({ length: 40 }, (_, index) => keysStep(String(index), "Ctrl+Shift+Alt+Win+A"));
    expect(macroByteEstimate(tooBig)).toBeGreaterThan(200);
    expect(validateKeyConfig(config({ route: "deviceMacro", macro: tooBig })).code).toBe("overLimit");
  });

  it("computes macro duration and dirty/conflict visuals", () => {
    const draft = config({ route: "host", macro: [keysStep("a", "A")], action: { kind: "launch", target: "a.exe", args: "", shell: "powershell", timeoutMs: 5000 } });
    const saved = config();
    expect(macroDuration(draft.macro)).toBe(50);
    const visual = stageVisual(draft, saved, true);
    expect(visual.conflict).toBe(true);
    expect(visual.dirty).toBe(true);
  });
});
