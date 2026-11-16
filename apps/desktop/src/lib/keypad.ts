import type { KeyConfig, KeyRoute, MacroStep } from "./types";

export type KeyApplyTarget = "device" | "host" | "both";

export interface StageKeyVisual {
  route: KeyRoute;
  configured: boolean;
  conflict: boolean;
  dirty: boolean;
}

export function isConfigured(config: KeyConfig): boolean {
  if (config.route === "devicePassthrough") return config.usage.trim().length > 0;
  if (config.route === "deviceMacro") return config.macro.length > 0;
  return config.action.target.trim().length > 0;
}

export function macroByteEstimate(macro: MacroStep[]): number {
  // P1 UI estimate only; the frozen encoder lives in kb-keymap (P2f).
  return 4 + macro.reduce((total, step) => {
    if (step.kind === "delay") return total + 3;
    const keys = step.keys.trim().length > 0 ? step.keys.split("+").length : 0;
    return total + 5 + keys * 2;
  }, 0);
}

export interface KeyValidation {
  ok: boolean;
  target: KeyApplyTarget;
  code?: "conflict" | "overLimit" | "targetRequired";
}

export function validateKeyConfig(config: KeyConfig): KeyValidation {
  const target: KeyApplyTarget = config.route === "host" ? "host" : "both";
  if (config.route === "host" && config.macro.length > 0) {
    return { ok: false, target, code: "conflict" };
  }
  if (macroByteEstimate(config.macro) > 200) {
    return { ok: false, target, code: "overLimit" };
  }
  if (config.route === "host" && config.action.target.trim().length === 0) {
    return { ok: false, target, code: "targetRequired" };
  }
  return { ok: true, target };
}

export function stageVisual(
  config: KeyConfig,
  saved: KeyConfig,
  _deviceOnline: boolean,
): StageKeyVisual {
  const validation = validateKeyConfig(config);
  return {
    route: config.route,
    configured: isConfigured(config),
    conflict: !validation.ok,
    dirty: JSON.stringify(config) !== JSON.stringify(saved),
  };
}

export function keyLabel(keyId: number): string {
  return `K${keyId}`;
}

export function macroDuration(macro: MacroStep[]): number {
  return macro.reduce((total, step) => total + step.holdMs + step.delayMs, 0);
}

export function nextTrigger(current: KeyConfig["trigger"]): KeyConfig["trigger"] {
  const order: KeyConfig["trigger"][] = ["tap", "double", "long", "repeat"];
  const index = order.indexOf(current);
  return order[(index + 1) % order.length] ?? "tap";
}
