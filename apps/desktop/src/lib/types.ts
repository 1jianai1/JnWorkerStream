import type { Locale } from "./i18n-core";

export type { Locale };

export type LinkState = "disconnected" | "connecting" | "connected" | "reconnecting" | "error";
export type TransportKind = "usb" | "ble";

export type LogLevel = "trace" | "debug" | "info" | "warn" | "error";
export type LogDomain = "link" | "frame" | "data" | "voice" | "stat" | "echo" | "rec" | "app" | "key";

export type VoicePhase =
  | "idle"
  | "recording"
  | "finalizing"
  | "queued"
  | "transcribing"
  | "injecting"
  | "done"
  | "failed";

export type InjectMode = "auto" | "sendinput" | "clipboard" | "recognize_only";
export type InjectOutcome = "sendinput" | "clipboard" | "blocked" | "none";

export interface LogEntry {
  at_ms: number;
  level: LogLevel;
  domain: LogDomain;
  /** i18n key used by the P1 mock source. Real backend entries use `text`. */
  textKey?: string;
  params?: Record<string, string | number>;
  text?: string;
  data?: Record<string, unknown>;
}

export interface ParserStats {
  frames: number;
  bytes: number;
  crc_err: number;
  oversize: number;
  unknown_type: number;
  resync: number;
  overflow: number;
}

export interface CheckStats {
  recv: number;
  lost: number;
  dup: number;
  order: number;
  pattern_err: number;
  length_err: number;
  length_changes: number;
  malformed: number;
  jitter_ms: number;
  mean_interval_ms: number;
  rate_fps: number;
  throughput_bps: number;
}

export interface TransportStats {
  rx_bytes: number;
  tx_bytes: number;
  errors: number;
  reconnects: number;
  last_error?: string;
}

export interface LinkInfo {
  state: LinkState;
  kind?: TransportKind;
  address?: string;
  mtu?: number;
  max_payload?: number;
  opened_at_ms?: number;
  reconnect_attempt?: number;
  next_retry_ms?: number;
  last_error?: string;
  last_error_detail?: string;
}

export interface VoiceState {
  enabled: boolean;
  inject_mode: InjectMode;
  phase: VoicePhase;
  seg_id?: number;
  started_at_ms?: number;
  elapsed_ms?: number;
  text?: string;
  language?: string;
  audio_ms?: number;
  latency_ms?: number;
  inject?: { mode: InjectMode; outcome: InjectOutcome; error?: string };
  queue_depth: number;
  segments: number;
  interrupted: number;
  failures: number;
  wav_path?: string;
  last_error?: string;
}

export interface RecordingState {
  state: "idle" | "recording" | "stopped" | "failed";
  path?: string;
  rows: number;
  bytes: number;
  last_error?: string;
}

export interface AsrStatus {
  ready: boolean;
  model_path?: string;
  device?: string;
  compute_type?: string;
  language?: string;
  last_error?: string;
}

export interface SidecarStatus {
  name: "ble" | "asr";
  state: "starting" | "ready" | "restarting" | "failed";
  version?: string;
  restarts: number;
  last_error?: string;
}

export type KeyRoute = "devicePassthrough" | "deviceMacro" | "host";
export type KeyTrigger = "tap" | "double" | "long" | "repeat";

export interface MacroStep {
  id: string;
  kind: "keys" | "delay";
  keys: string;
  usage?: number;
  modifier?: number;
  holdMs: number;
  delayMs: number;
}

export type ActionKind = "launch" | "openPath" | "openUrl" | "script";

export interface HostAction {
  kind: ActionKind;
  target: string;
  args: string;
  shell: "powershell" | "pwsh" | "cmd";
  timeoutMs: number;
}

export interface KeyConfig {
  route: KeyRoute;
  report: boolean;
  disabled: boolean;
  usage: string;
  modifier: string;
  /** Captured display label (e.g. "Ctrl+C"), UI-only in P1. */
  captureLabel?: string;
  macro: MacroStep[];
  action: HostAction;
  trigger: KeyTrigger;
}

export interface KeysState {
  device_online: boolean;
  pending_push: number;
  last_applied_at_ms?: number;
  last_target?: "device" | "host" | "both";
  configs: KeyConfig[];
}

export interface AppSettings {
  schema_version: number;
  link: {
    kind: TransportKind;
    auto_reconnect: boolean;
    usb: { device: string };
    ble: { name: string; paired_only: boolean };
  };
  audio: {
    receive: boolean;
    save_dir: string;
    playback: boolean;
    buf_ms: number;
  };
  voice: {
    enabled: boolean;
    inject_mode: InjectMode;
    asr: { model_path: string; device: "cpu" | "cuda"; compute_type: string; language: string };
  };
  logging: { level_min: LogLevel; max_entries: number };
  window: { width: number; height: number };
  ui: {
    theme: "light";
    locale: Locale;
    keypad: { renderer: "auto" | "3d" | "2d" };
  };
}

export interface AppState {
  revision: number;
  app: { version: string; os: string; started_at_ms: number };
  link: LinkInfo;
  parser: ParserStats;
  check: CheckStats;
  transport: TransportStats;
  queue: { frame_depth: number; frame_capacity: number; log_depth: number; log_capacity: number };
  voice: VoiceState;
  asr: AsrStatus;
  recording: RecordingState;
  sidecars: SidecarStatus[];
  keys: KeysState;
  settings: AppSettings;
  logs: LogEntry[];
  dropped_logs: number;
}

export type DeepPartial<T> = {
  [K in keyof T]?: T[K] extends object ? DeepPartial<T[K]> : T[K];
};
