import type {
  AppSettings,
  AppState,
  DeepPartial,
  HostAction,
  KeyConfig,
  KeysState,
  LogDomain,
  LogEntry,
  LogLevel,
  MacroStep,
  TransportKind,
  VoiceState,
} from "../lib/types";
import { DEFAULT_LOCALE, isLocale, type Locale } from "../lib/i18n-core";

export type Scenario =
  | "default"
  | "connected"
  | "link-error"
  | "reconnecting"
  | "offline"
  | "voice-failed"
  | "voice-done"
  | "long-logs"
  | "empty"
  | "corrupt-settings";

export interface ConfigIssue {
  corrupt: boolean;
  backup?: string;
}

const SETTINGS_KEY = "jnws.p1.settings";
const KEYS_KEY = "jnws.p1.keys";

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
    asr: { model_path: "models/faster-whisper-small", device: "cpu", compute_type: "int8", language: "zh" },
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

function readStoredSettings(): { settings: AppSettings; issue: ConfigIssue } {
  try {
    const raw = localStorage.getItem(SETTINGS_KEY);
    if (!raw) return { settings: cloneSettings(DEFAULT_SETTINGS), issue: { corrupt: false } };
    const parsed = JSON.parse(raw) as DeepPartial<AppSettings>;
    const settings = mergeDeep(cloneSettings(DEFAULT_SETTINGS), parsed);
    // Hardware only exposes USB CDC and BLE. Older P1 drafts used serial/sim.
    if (settings.link.kind !== "usb" && settings.link.kind !== "ble") {
      settings.link.kind = "usb";
    }
    return { settings, issue: { corrupt: false } };
  } catch {
    return {
      settings: cloneSettings(DEFAULT_SETTINGS),
      issue: { corrupt: true, backup: "config.corrupt-1763200000000.json" },
    };
  }
}

const emptyMacro: MacroStep[] = [];
const defaultAction: HostAction = {
  kind: "launch",
  target: "",
  args: "",
  shell: "powershell",
  timeoutMs: 5000,
};

export function makeDefaultKeyConfig(keyId: number): KeyConfig {
  const base: KeyConfig = {
    route: "devicePassthrough",
    report: false,
    disabled: false,
    usage: "",
    modifier: "none",
    macro: [...emptyMacro],
    action: { ...defaultAction },
    trigger: "tap",
  };
  if (keyId === 7) {
    return {
      ...base,
      route: "deviceMacro",
      macro: [
        { id: "s1", kind: "keys", keys: "Ctrl+C", usage: 0x06, modifier: 0x01, holdMs: 20, delayMs: 0 },
        { id: "s2", kind: "keys", keys: "Ctrl+V", usage: 0x19, modifier: 0x01, holdMs: 20, delayMs: 30 },
      ],
    };
  }
  if (keyId === 9) {
    return {
      ...base,
      route: "host",
      action: { kind: "launch", target: "C:\\Windows\\System32\\notepad.exe", args: "", shell: "powershell", timeoutMs: 5000 },
    };
  }
  if (keyId === 3) {
    return { ...base, report: true, usage: "0x1D", modifier: "0x01", captureLabel: "Ctrl+Z" };
  }
  return base;
}

function readStoredKeys(): KeyConfig[] {
  try {
    const raw = localStorage.getItem(KEYS_KEY);
    if (!raw) return Array.from({ length: 16 }, (_, index) => makeDefaultKeyConfig(index));
    const parsed = JSON.parse(raw) as KeyConfig[];
    if (!Array.isArray(parsed) || parsed.length !== 16) throw new Error("bad keys");
    return parsed;
  } catch {
    return Array.from({ length: 16 }, (_, index) => makeDefaultKeyConfig(index));
  }
}

let logSeq = 0;

function log(level: LogLevel, domain: LogDomain, textKey: string, params?: Record<string, string | number>): LogEntry {
  logSeq += 1;
  return {
    at_ms: Date.now() - (2000 - logSeq) * 37,
    level,
    domain,
    textKey,
    params,
  };
}

function seedLogs(count: number, _scenario: Scenario): LogEntry[] {
  if (count <= 0) return [];
  const levels: LogLevel[] = ["trace", "debug", "info", "info", "info", "warn", "error", "info"];
  const domains: LogDomain[] = ["frame", "data", "link", "stat", "voice", "echo", "app", "key"];
  const now = Date.now();
  const entries: LogEntry[] = [];
  for (let index = 0; index < count; index += 1) {
    const position = index + 1;
    const rawLevel = levels[(index * 7 + 3) % levels.length] ?? "info";
    const level: LogLevel = _scenario === "long-logs"
      ? rawLevel
      : rawLevel === "error" || rawLevel === "warn"
        ? "info"
        : rawLevel;
    const domain = domains[(index * 5 + 1) % domains.length] ?? "frame";
    const seq = ((position * 3) % 65535) + 1;
    let textKey = "mock.log.frame";
    let params: Record<string, string | number> = { seq, type: "60", len: 64 + (position % 7) };
    if (domain === "stat") {
      textKey = "mock.log.stats";
      params = { fps: 20, kbps: 1.5, lost: 0 };
    } else if (level === "warn") {
      textKey = "mock.log.warnQueue";
      params = { percent: 82 };
    } else if (level === "error") {
      textKey = "mock.log.crc";
      params = { count: Math.floor(position / 8) + 1 };
    } else if (level === "debug") {
      textKey = "mock.log.debugParser";
      params = { size: 128 + (position % 64) };
    } else if (level === "trace") {
      textKey = "mock.log.traceByte";
      params = { hex: "A5 5A 01 60 00" };
    } else if (domain === "voice") {
      textKey = "mock.log.voiceSeg";
      params = { seg: position % 200, ms: 1200 };
    } else if (domain === "echo") {
      textKey = "mock.log.echo";
      params = { seq };
    } else if (domain === "app") {
      textKey = "mock.log.appStart";
      params = { schema: 1 };
    } else if (domain === "key") {
      textKey = "mock.log.cfgApplied";
      params = { key: `K${position % 16}`, target: "device" };
    }
    entries.push({
      at_ms: now - (count - index) * 137,
      level,
      domain,
      textKey,
      params,
    });
  }
  return entries;
}

function initialCounters(scenario: Scenario) {
  if (["connected", "offline", "reconnecting", "voice-failed", "voice-done", "long-logs"].includes(scenario)) {
    return {
      parser: { frames: 18432, bytes: 1_925_120, crc_err: 0, oversize: 0, unknown_type: 2, resync: 3, overflow: 0 },
      check: {
        recv: 18432,
        lost: 0,
        dup: 0,
        order: 0,
        pattern_err: 0,
        length_err: 0,
        length_changes: 0,
        malformed: 0,
        jitter_ms: 0.82,
        mean_interval_ms: 50,
        rate_fps: 20,
        throughput_bps: 1500,
      },
      transport: { rx_bytes: 1_925_120, tx_bytes: 12_288, errors: 0, reconnects: 0 },
    };
  }
  return {
    parser: { frames: 0, bytes: 0, crc_err: 0, oversize: 0, unknown_type: 0, resync: 0, overflow: 0 },
    check: {
      recv: 0,
      lost: 0,
      dup: 0,
      order: 0,
      pattern_err: 0,
      length_err: 0,
      length_changes: 0,
      malformed: 0,
      jitter_ms: 0,
      mean_interval_ms: 0,
      rate_fps: 0,
      throughput_bps: 0,
    },
    transport: { rx_bytes: 0, tx_bytes: 0, errors: 0, reconnects: 0 },
  };
}

function initialVoice(scenario: Scenario, enabled: boolean, locale: Locale): VoiceState {
  const base: VoiceState = {
    enabled,
    inject_mode: "auto",
    phase: "idle",
    queue_depth: 0,
    segments: 0,
    interrupted: 0,
    failures: 0,
  };
  if (scenario === "voice-done") {
    return {
      ...base,
      enabled: true,
      phase: "done",
      seg_id: 42,
      text: locale === "zh-CN"
        ? "这是一段来自设备按键的本地识别结果。"
        : "This is a local transcription from a device key press.",
      language: locale === "zh-CN" ? "zh" : "en",
      audio_ms: 1640,
      latency_ms: 1240,
      elapsed_ms: 1640,
      segments: 3,
      wav_path: "logs/voice/seg_20261115_214012_42.wav",
      inject: { mode: "auto", outcome: "sendinput" },
    };
  }
  if (scenario === "voice-failed") {
    return {
      ...base,
      enabled: true,
      phase: "failed",
      failures: 1,
      segments: 1,
      seg_id: 41,
      audio_ms: 1640,
      wav_path: "logs/voice/seg_20261115_214012_41.wav",
      last_error: "E_ASR_TIMEOUT",
      elapsed_ms: 30_000,
    };
  }
  return base;
}

export function buildInitialState(scenario: Scenario): { state: AppState; issue: ConfigIssue } {
  const { settings: stored, issue } = readStoredSettings();
  if (scenario === "corrupt-settings") {
    issue.corrupt = true;
    issue.backup = "config.corrupt-1763200000000.json";
  }
  const params = new URLSearchParams(window.location.search);
  const forced = params.get("lang");
  if (isLocale(forced)) stored.ui.locale = forced;
  const locale: Locale = stored.ui.locale;

  let link: AppState["link"];
  const kind: TransportKind = stored.link.kind;
  if (scenario === "link-error") {
    link = {
      state: "error",
      kind,
      address: kind === "ble" ? "SF32" : "USB CDC · COM3",
      last_error: "E_PORT_BUSY",
      last_error_detail: "Access denied (COM3)",
    };
  } else if (scenario === "offline") {
    link = {
      state: "error",
      kind,
      address: kind === "ble" ? "SF32" : "USB CDC · COM3",
      last_error: "E_DEVICE_OFFLINE",
      last_error_detail: "heartbeat timeout (5 s)",
    };
  } else if (scenario === "connected" || scenario === "voice-failed" || scenario === "voice-done" || scenario === "long-logs") {
    link = {
      state: "connected",
      kind,
      address: kind === "ble" ? "SF32" : "USB CDC · COM3",
      mtu: kind === "ble" ? 247 : 1024,
      max_payload: kind === "ble" ? 233 : 1013,
      opened_at_ms: Date.now() - 12 * 60 * 1000,
    };
  } else if (scenario === "reconnecting") {
    link = {
      state: "reconnecting",
      kind,
      address: kind === "ble" ? "SF32" : "USB CDC · COM3",
      reconnect_attempt: 2,
      next_retry_ms: 4000,
      last_error: "E_WRITE_FAILED",
      last_error_detail: "link lost",
    };
  } else {
    link = { state: "disconnected", kind };
  }

  const longLogs = scenario === "long-logs";
  const logCount = scenario === "empty" ? 0 : longLogs ? 5000 : 160;
  const keysState: KeysState = {
    device_online: link.state === "connected" && scenario !== "offline",
    pending_push: scenario === "offline" ? 1 : 0,
    configs: readStoredKeys(),
    last_applied_at_ms: link.state === "connected" ? Date.now() - 8 * 60 * 1000 : undefined,
    last_target: link.state === "connected" ? "both" : undefined,
  };

  const initialLogs = seedLogs(logCount, scenario);
  if (scenario === "default") {
    initialLogs.splice(0, initialLogs.length,
      log("info", "app", "mock.log.appStart", { schema: 1 }),
      log("info", "app", "mock.log.mockMode"),
      log("info", "app", "mock.log.appReady"),
    );
  }
  if (initialLogs.length === 0 && scenario !== "empty") {
    initialLogs.push(log("info", "app", "mock.log.appStart", { schema: 1 }));
  }
  const counters = initialCounters(scenario);
  counters.parser.crc_err = initialLogs.filter((entry) => entry.textKey === "mock.log.crc").length;

  return {
    issue,
    state: {
      revision: 1,
      app: { version: "0.1.0-p1", os: "Windows 11", started_at_ms: Date.now() - 42 * 60 * 1000 },
      link,
      ...counters,
      queue: {
        frame_depth: scenario === "long-logs"
          ? 8200
          : ["connected", "offline", "voice-done", "voice-failed"].includes(scenario)
            ? 12
            : 0,
        frame_capacity: 10000,
        log_depth: initialLogs.length,
        log_capacity: 20000,
      },
      voice: initialVoice(scenario, scenario === "voice-failed" || scenario === "voice-done", locale),
      asr: {
        ready: scenario !== "voice-failed",
        model_path: "models/faster-whisper-small",
        device: "cpu",
        compute_type: "int8",
        language: locale === "zh-CN" ? "zh" : "en",
        last_error: scenario === "voice-failed" ? "E_ASR_TIMEOUT" : undefined,
      },
      recording: { state: "idle", path: "logs/record_20261115_210000.csv", rows: 0, bytes: 0 },
      sidecars: [
        { name: "ble", state: "ready", version: "0.1.0", restarts: 0 },
        { name: "asr", state: scenario === "voice-failed" ? "failed" : "ready", version: "0.1.0", restarts: scenario === "voice-failed" ? 1 : 0 },
      ],
      keys: keysState,
      settings: stored,
      logs: initialLogs,
      dropped_logs: scenario === "long-logs" ? 12 : 0,
    },
  };
}

export function isScenario(value: string | null): value is Scenario {
  return (
    value === "default" ||
    value === "connected" ||
    value === "link-error" ||
    value === "reconnecting" ||
    value === "offline" ||
    value === "voice-failed" ||
    value === "voice-done" ||
    value === "long-logs" ||
    value === "empty" ||
    value === "corrupt-settings"
  );
}

export function persistSettings(settings: AppSettings): void {
  try {
    localStorage.setItem(SETTINGS_KEY, JSON.stringify(settings));
  } catch {
    /* storage is best-effort in the mock */
  }
}

export function persistKeys(configs: KeyConfig[]): void {
  try {
    localStorage.setItem(KEYS_KEY, JSON.stringify(configs));
  } catch {
    /* storage is best-effort in the mock */
  }
}

export function nextLocaleFor(locale: Locale): Locale {
  return locale === "zh-CN" ? "en-US" : "zh-CN";
}
