import { DEFAULT_LOCALE } from "./i18n-core";
import type { AppSettings, DeepPartial } from "./types";

/** 设置持久化键（与 P1 Mock 共用，切真实后端时沿用同一份本地设置）。 */
export const SETTINGS_KEY = "jnws.p1.settings";

export const DEFAULT_SETTINGS: AppSettings = {
  schema_version: 1,
  link: {
    kind: "usb",
    auto_reconnect: true,
    usb: { device: "USB CDC · COM3" },
    ble: { name: "SF32", paired_only: true },
  },
  audio: {
    receive: false,
    save_dir: "logs/voice",
    playback: true,
    buf_ms: 20,
  },
  voice: {
    enabled: false,
    inject_mode: "auto",
    asr: {
      model_path: "models/faster-whisper-small",
      device: "cpu",
      compute_type: "int8",
      language: "zh",
    },
  },
  logging: { level_min: "info", max_entries: 5000 },
  window: { width: 1280, height: 800 },
  ui: {
    theme: "light",
    locale: DEFAULT_LOCALE,
    keypad: { renderer: "auto" },
  },
};

export function mergeDeep<T>(base: T, patch: DeepPartial<T> | undefined): T {
  if (!patch) return base;
  const output = { ...base } as Record<string, unknown>;
  for (const [key, value] of Object.entries(patch)) {
    if (value === undefined) continue;
    const current = output[key];
    const currentRecord = current as Record<string, unknown> | undefined;
    const nextRecord = value as Record<string, unknown> | undefined;
    if (
      currentRecord &&
      nextRecord &&
      typeof currentRecord === "object" &&
      typeof nextRecord === "object" &&
      !Array.isArray(currentRecord) &&
      !Array.isArray(nextRecord)
    ) {
      output[key] = mergeDeep(currentRecord, nextRecord as DeepPartial<typeof currentRecord>);
    } else {
      output[key] = value;
    }
  }
  return output as T;
}

export function cloneSettings(settings: AppSettings): AppSettings {
  return JSON.parse(JSON.stringify(settings)) as AppSettings;
}

export function persistSettings(settings: AppSettings): void {
  try {
    localStorage.setItem(SETTINGS_KEY, JSON.stringify(settings));
  } catch {
    /* 存储是尽力而为：隐私模式或测试环境可能不可用 */
  }
}
