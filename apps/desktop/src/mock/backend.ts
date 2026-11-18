import type { ApplyResult, Backend, ConfigIssue } from "../lib/backend";
import type {
  AppSettings,
  AppState,
  DeepPartial,
  InjectMode,
  KeyConfig,
  LogDomain,
  LogEntry,
  LogLevel,
  TransportKind,
} from "../lib/types";
import {
  DEFAULT_SETTINGS,
  buildInitialState,
  cloneSettings,
  mergeDeep,
  persistKeys,
  persistSettings,
  type Scenario,
} from "./seed";

function kindAddress(kind: TransportKind): string {
  return kind === "ble" ? "SF32" : "USB CDC · COM3";
}

function kindMtu(kind: TransportKind): { mtu: number; max_payload: number } {
  if (kind === "ble") return { mtu: 247, max_payload: 233 };
  return { mtu: 1024, max_payload: 1013 };
}

const MOCK_RATE_HZ = 20;
const MOCK_PAYLOAD_BYTES = 64;

export class MockBackend implements Backend {
  private state: AppState;
  private issue: ConfigIssue;
  private readonly listeners = new Set<() => void>();
  private statsTimer?: number;
  private logTimer?: number;
  private reconnectTimer?: number;
  private connectTimer?: number;
  private voiceElapsedTimer?: number;
  private voiceTimers: number[] = [];
  private tick = 0;
  private failNext: boolean;

  constructor(scenario: Scenario = "default") {
    const initial = buildInitialState(scenario);
    this.state = initial.state;
    this.issue = initial.issue;
    this.failNext = scenario === "voice-failed";
    if (this.state.link.state === "connected") this.startTimers();
    if (scenario === "reconnecting") {
      this.reconnectTimer = window.setTimeout(() => this.establish(false), 4000);
    }
  }

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  getState = (): AppState => this.state;

  getConfigIssue = (): ConfigIssue => this.issue;

  private commit(patch: Partial<AppState>): void {
    this.state = { ...this.state, ...patch, revision: this.state.revision + 1 };
    for (const listener of this.listeners) listener();
  }

  private addLog(level: LogLevel, domain: LogDomain, textKey: string, params?: Record<string, string | number>): void {
    const entry: LogEntry = { at_ms: Date.now(), level, domain, textKey, params };
    const max = this.state.settings.logging.max_entries;
    const merged = [...this.state.logs, entry];
    const overflow = Math.max(0, merged.length - max);
    const logs = overflow > 0 ? merged.slice(overflow) : merged;
    this.commit({
      logs,
      dropped_logs: this.state.dropped_logs + overflow,
      queue: { ...this.state.queue, log_depth: logs.length },
    });
  }

  private startTimers(): void {
    this.stopTimers();
    this.statsTimer = window.setInterval(() => this.statsTick(), 1000);
    this.logTimer = window.setInterval(() => this.logTick(), 1600);
  }

  private stopTimers(): void {
    if (this.statsTimer !== undefined) window.clearInterval(this.statsTimer);
    if (this.logTimer !== undefined) window.clearInterval(this.logTimer);
    this.statsTimer = undefined;
    this.logTimer = undefined;
  }

  private statsTick(): void {
    if (this.state.link.state !== "connected") return;
    this.tick += 1;
    const rate = MOCK_RATE_HZ;
    const size = MOCK_PAYLOAD_BYTES;
    const frameDelta = Math.max(1, Math.round(rate + (this.tick % 3) - 1));
    const byteDelta = frameDelta * (size + 11);
    const parser = this.state.parser;
    const check = this.state.check;
    const transport = this.state.transport;
    const frameDepth = 8 + ((this.tick * 3) % 11);
    this.commit({
      parser: { ...parser, frames: parser.frames + frameDelta, bytes: parser.bytes + byteDelta },
      check: {
        ...check,
        recv: check.recv + frameDelta,
        jitter_ms: Number((0.72 + ((this.tick * 7) % 5) * 0.06).toFixed(2)),
        mean_interval_ms: 50,
        rate_fps: frameDelta,
        throughput_bps: byteDelta,
      },
      transport: { ...transport, rx_bytes: transport.rx_bytes + byteDelta, tx_bytes: transport.tx_bytes + 32 },
      queue: { ...this.state.queue, frame_depth: frameDepth, log_depth: this.state.logs.length },
    });
  }

  private logTick(): void {
    if (this.state.link.state !== "connected") return;
    this.tick += 1;
    const seq = 1000 + this.tick * MOCK_RATE_HZ;
    const kbps = ((MOCK_RATE_HZ * (MOCK_PAYLOAD_BYTES + 11)) / 1024).toFixed(1);
    const variants: Array<() => void> = [
      () => this.addLog("info", "frame", "mock.log.frame", { seq, type: "60", len: 64 }),
      () => this.addLog("debug", "data", "mock.log.debugParser", { size: 128 }),
      () => this.addLog("info", "stat", "mock.log.stats", { fps: 20, kbps, lost: 0 }),
      () => this.addLog("info", "echo", "mock.log.echo", { seq }),
      () => this.addLog("warn", "frame", "mock.log.warnQueue", { percent: 82 }),
    ];
    const variant = variants[this.tick % variants.length];
    variant?.();
  }

  private establish(countReconnect: boolean): void {
    const kind = this.state.settings.link.kind;
    const { mtu, max_payload } = kindMtu(kind);
    const address = kindAddress(kind);
    const reconnects = this.state.transport.reconnects + (countReconnect ? 1 : 0);
    this.commit({
      link: {
        state: "connected",
        kind,
        address,
        mtu,
        max_payload,
        opened_at_ms: Date.now(),
      },
      transport: { ...this.state.transport, reconnects, last_error: undefined },
    });
    this.startTimers();
    this.addLog("info", "link", "mock.log.linkUp", { kind, address, mtu });
    this.addLog("info", "frame", "mock.log.frame", { seq: 0, type: "60", len: 64 });
    this.addLog("info", "stat", "mock.log.stats", { fps: 20, kbps: 1.5, lost: 0 });
    this.addLog("info", "echo", "mock.log.echo", { seq: 0 });
  }

  connect = async (): Promise<void> => {
    if (this.state.link.state === "connected" || this.state.link.state === "connecting") return;
    const kind = this.state.settings.link.kind;
    const address = kindAddress(kind);
    this.commit({ link: { state: "connecting", kind, address } });
    this.addLog("info", "link", "mock.log.linkConnecting", { kind, address });
    this.connectTimer = window.setTimeout(() => this.establish(false), 800);
  };

  cancelConnect = async (): Promise<void> => {
    if (this.state.link.state !== "connecting") return;
    if (this.connectTimer !== undefined) window.clearTimeout(this.connectTimer);
    this.connectTimer = undefined;
    this.commit({ link: { state: "disconnected", kind: this.state.settings.link.kind } });
    this.addLog("warn", "link", "mock.log.linkDown", { reason: "连接已取消" });
  };

  disconnect = async (): Promise<void> => {
    this.stopTimers();
    this.clearVoiceTimers();
    this.commit({
      link: { state: "disconnected", kind: this.state.settings.link.kind },
      voice: { ...this.state.voice, phase: "idle", queue_depth: 0 },
    });
    this.addLog("info", "link", "mock.log.linkDown", { reason: "用户断开" });
  };

  retry = async (): Promise<void> => {
    this.commit({ link: { state: "connecting", kind: this.state.settings.link.kind } });
    window.setTimeout(() => this.establish(false), 700);
  };

  simulateLinkLoss = async (): Promise<void> => {
    if (this.state.link.state !== "connected") return;
    this.stopTimers();
    const kind = this.state.settings.link.kind;
    this.addLog("warn", "link", "mock.log.linkDown", { reason: "模拟断连" });
    if (!this.state.settings.link.auto_reconnect) {
      this.commit({
        link: {
          state: "error",
          kind,
          last_error: "E_WRITE_FAILED",
          last_error_detail: "link lost",
        },
      });
      return;
    }
    this.commit({
      link: {
        state: "reconnecting",
        kind,
        reconnect_attempt: 1,
        next_retry_ms: 2000,
        last_error: "E_WRITE_FAILED",
        last_error_detail: "link lost",
      },
    });
    this.reconnectTimer = window.setTimeout(() => this.establish(true), 2000);
  };

  stopReconnect = async (): Promise<void> => {
    if (this.reconnectTimer !== undefined) window.clearTimeout(this.reconnectTimer);
    this.reconnectTimer = undefined;
    this.commit({ link: { state: "disconnected", kind: this.state.settings.link.kind } });
  };

  updateSettings = async (patch: DeepPartial<AppSettings>): Promise<AppSettings> => {
    const settings = mergeDeep(cloneSettings(this.state.settings), patch);
    const trimmed = this.state.logs.length > settings.logging.max_entries
      ? this.state.logs.slice(-settings.logging.max_entries)
      : this.state.logs;
    persistSettings(settings);
    this.commit({
      settings,
      logs: trimmed,
      queue: { ...this.state.queue, log_depth: trimmed.length },
    });
    return settings;
  };

  resetSettings = async (): Promise<AppSettings> => {
    const settings = cloneSettings(DEFAULT_SETTINGS);
    persistSettings(settings);
    this.commit({ settings });
    return settings;
  };

  clearLogs = async (): Promise<void> => {
    this.commit({ logs: [], dropped_logs: 0, queue: { ...this.state.queue, log_depth: 0 } });
  };

  setVoiceEnabled = async (enabled: boolean): Promise<void> => {
    if (!enabled) {
      this.clearVoiceTimers();
      this.commit({ voice: { ...this.state.voice, enabled: false, phase: "idle", queue_depth: 0 } });
      return;
    }
    this.commit({ voice: { ...this.state.voice, enabled: true } });
  };

  setInjectMode = async (mode: InjectMode): Promise<void> => {
    this.commit({ voice: { ...this.state.voice, inject_mode: mode } });
  };

  simulatePtt = async (): Promise<void> => {
    if (!this.state.voice.enabled) return;
    if (["recording", "finalizing", "transcribing"].includes(this.state.voice.phase)) return;
    this.clearVoiceTimers();
    const seg = (this.state.voice.seg_id ?? 40) + 1;
    const startedAt = Date.now();
    this.commit({
      voice: {
        ...this.state.voice,
        phase: "recording",
        seg_id: seg,
        started_at_ms: startedAt,
        elapsed_ms: 0,
        text: undefined,
        inject: undefined,
        latency_ms: undefined,
        audio_ms: undefined,
        wav_path: undefined,
        last_error: undefined,
        queue_depth: 1,
      },
    });
    this.addLog("info", "voice", "mock.log.voiceSeg", { seg, ms: 1600 });
    this.voiceElapsedTimer = window.setInterval(() => {
      this.commit({
        voice: { ...this.state.voice, elapsed_ms: Date.now() - startedAt },
      });
    }, 200);
    this.voiceTimers.push(
      window.setTimeout(() => this.commit({ voice: { ...this.state.voice, phase: "finalizing" } }), 1600),
      window.setTimeout(() => this.commit({ voice: { ...this.state.voice, phase: "transcribing" } }), 1900),
      window.setTimeout(() => this.finishVoice(seg), 3400),
    );
  };

  private finishVoice(seg: number): void {
    this.clearVoiceTimers();
    if (this.failNext) {
      const wav = `logs/voice/seg_${new Date().toISOString().replace(/[-:TZ.]/g, "").slice(0, 14)}_${seg}.wav`;
      this.commit({
        voice: {
          ...this.state.voice,
          phase: "failed",
          queue_depth: 0,
          failures: this.state.voice.failures + 1,
          last_error: "E_ASR_TIMEOUT",
          wav_path: wav,
          elapsed_ms: 30_000,
        },
      });
      this.addLog("error", "voice", "mock.log.voiceFail", { seg, code: "E_ASR_TIMEOUT" });
      return;
    }
    const zh = this.state.settings.ui.locale === "zh-CN";
    const text = zh
      ? "这是一段来自设备按键的本地识别结果。"
      : "This is a local transcription from a device key press.";
    const mode = this.state.voice.inject_mode;
    const outcome = mode === "recognize_only" ? "none" : mode === "clipboard" ? "clipboard" : "sendinput";
    this.commit({
      voice: {
        ...this.state.voice,
        phase: "done",
        queue_depth: 0,
        text,
        language: zh ? "zh" : "en",
        audio_ms: 1640,
        latency_ms: 1240,
        elapsed_ms: 1640,
        segments: this.state.voice.segments + 1,
        wav_path: "logs/voice/seg_latest.wav",
        inject: { mode, outcome },
      },
    });
    this.addLog("info", "voice", "mock.log.asrDone", { seg, ms: 1240, mode: outcome });
  }

  cancelVoice = async (): Promise<void> => {
    if (!["recording", "finalizing", "transcribing", "queued"].includes(this.state.voice.phase)) return;
    this.clearVoiceTimers();
    this.commit({
      voice: {
        ...this.state.voice,
        phase: "idle",
        queue_depth: 0,
        interrupted: this.state.voice.interrupted + 1,
      },
    });
  };

  retryVoice = async (): Promise<void> => {
    if (this.state.voice.phase !== "failed") return;
    this.failNext = false;
    this.commit({
      voice: { ...this.state.voice, phase: "transcribing", queue_depth: 1, last_error: undefined },
    });
    this.voiceTimers.push(window.setTimeout(() => this.finishVoice(this.state.voice.seg_id ?? 42), 1200));
  };

  applyKey = async (keyId: number, config: KeyConfig): Promise<ApplyResult> => {
    const target = config.route === "host" ? "host" : "both";
    const configs = [...this.state.keys.configs];
    configs[keyId] = config;
    persistKeys(configs);
    const online = this.state.keys.device_online;
    const pending = online ? Math.max(0, this.state.keys.pending_push - 1) : this.state.keys.pending_push + 1;
    this.commit({
      keys: {
        ...this.state.keys,
        configs,
        pending_push: pending,
        last_applied_at_ms: online ? Date.now() : this.state.keys.last_applied_at_ms,
        last_target: online ? target : "host",
      },
    });
    if (online) {
      this.addLog("info", "key", "mock.log.cfgApplied", { key: `K${keyId}`, target });
    }
    return { target, applied: online, pending };
  };

  private clearVoiceTimers(): void {
    if (this.voiceElapsedTimer !== undefined) window.clearInterval(this.voiceElapsedTimer);
    this.voiceElapsedTimer = undefined;
    for (const timer of this.voiceTimers) window.clearTimeout(timer);
    this.voiceTimers = [];
  }
}
