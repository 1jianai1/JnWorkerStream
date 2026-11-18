import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type { ApplyResult, Backend, ConfigIssue } from "./backend";
import type {
  AppSettings,
  AppState,
  DeepPartial,
  InjectMode,
  InjectOutcome,
  KeyConfig,
  LinkState,
  LogDomain,
  LogEntry,
  LogLevel,
  VoicePhase,
} from "./types";
import {
  DEFAULT_SETTINGS,
  cloneSettings,
  mergeDeep,
  persistSettings,
} from "./settings";
import { buildInitialState } from "../mock/seed";

/** `app.snapshot` 的返回（`crates/kb-app` 的 `Snapshot`，字段名对齐 interfaces.md §2.4）。 */
interface SnapshotDto {
  revision: number;
  link: {
    state: string;
    kind?: string;
    address?: string;
    mtu?: number;
    max_payload?: number;
    last_error?: string;
    reconnect_attempt?: number;
    next_retry_ms?: number;
  };
  parser: {
    frames: number;
    bytes: number;
    crc_err: number;
    oversize: number;
    unknown_type: number;
    resync: number;
    overflow: number;
  };
  transport: {
    rx_bytes: number;
    tx_bytes: number;
    errors: number;
    reconnects: number;
    dropped: number;
    last_error?: string;
  };
  voice: {
    enabled: boolean;
    segments: number;
    interrupted: number;
    failures: number;
    last_text?: string;
  };
}

interface LinkStateEvent {
  revision: number;
  state: string;
  kind?: string;
  address?: string;
  mtu?: number;
  max_payload?: number;
  last_error?: string;
  reconnect_attempt?: number;
  next_retry_ms?: number;
}

interface FrameBatchEvent {
  revision: number;
  dropped: number;
  frames: Array<{ seq: number; type: number; flags: number; len: number; at_ms: number }>;
}

interface VoiceResultEvent {
  revision: number;
  seg_id: number;
  text: string;
  language: string;
  audio_ms: number;
  latency_ms: number;
  inject: { mode: string; ok: boolean; error?: string };
  note?: string;
}

interface AppErrorEvent {
  revision: number;
  message: string;
}

const LINK_STATES: Record<string, LinkState> = {
  idle: "disconnected",
  connecting: "connecting",
  connected: "connected",
  reconnecting: "reconnecting",
  disconnected: "disconnected",
  error: "error",
};

const MAX_LOGS = 2000;

function commandError(error: unknown): string {
  if (error && typeof error === "object" && "code" in error && "message" in error) {
    const detail = error as { code: string; message: string };
    return `${detail.code}: ${detail.message}`;
  }
  return String(error);
}

/**
 * 真实后端：把 `kb-app` 的 snapshot/事件映射成 UI 的 `AppState`。
 *
 * - 连接/语音动作走 Tauri 命令；`kb-app` 每 10 ms 轮询传输层并 emit 事件；
 * - `check` / `queue` / `keys` / `recording` 仍由前端占位（对应 Rust 侧尚未实现的部分）；
 * - 设置暂存 localStorage，并只在 Rust 有对应命令时下推（`link.reconnect_policy`）。
 */
export class TauriBackend implements Backend {
  private state: AppState;
  private readonly listeners = new Set<() => void>();
  private unlisten: UnlistenFn[] = [];
  private refreshTimer?: number;
  private disposed = false;

  constructor() {
    const initial = buildInitialState("default");
    this.state = { ...initial.state, link: { state: "disconnected" } };
    void this.bootstrap();
  }

  private async bootstrap(): Promise<void> {
    // 事件名不能带点号：Tauri 2 只允许字母数字和 `- / : _`，带 `.` 的 listen/emit
    // 会直接报 IllegalEventName（ADR-021）。这里用连字符名，且用 allSettled
    // 保证一个事件订阅失败不影响其余事件。
    const settled = await Promise.allSettled([
      listen<LinkStateEvent>("link-state", (event) => this.onLinkState(event.payload)),
      listen<FrameBatchEvent>("frame-batch", (event) => this.onFrameBatch(event.payload)),
      listen<VoiceResultEvent>("voice-result", (event) => this.onVoiceResult(event.payload)),
      listen<AppErrorEvent>("app-error", (event) => this.onAppError(event.payload)),
    ]);
    for (const result of settled) {
      if (result.status === "fulfilled") {
        this.unlisten.push(result.value);
      } else {
        this.addLog("error", "app", `事件订阅失败：${commandError(result.reason)}`);
      }
    }
    await this.refresh();
    this.refreshTimer = window.setInterval(() => void this.refresh(), 1000);
  }

  dispose(): void {
    this.disposed = true;
    if (this.refreshTimer !== undefined) window.clearInterval(this.refreshTimer);
    this.refreshTimer = undefined;
    for (const un of this.unlisten) un();
    this.unlisten = [];
  }

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  getState = (): AppState => this.state;

  getConfigIssue = (): ConfigIssue => ({ corrupt: false });

  // -- 状态更新 ---------------------------------------------------------

  private commit(patch: Partial<AppState>): void {
    this.state = { ...this.state, ...patch, revision: this.state.revision + 1 };
    for (const listener of this.listeners) listener();
  }

  private addLog(
    level: LogLevel,
    domain: LogDomain,
    text: string,
    data?: Record<string, unknown>,
  ): void {
    const entry: LogEntry = { at_ms: Date.now(), level, domain, text, data };
    const logs = [...this.state.logs, entry];
    const overflow = Math.max(0, logs.length - MAX_LOGS);
    this.commit({
      logs: overflow > 0 ? logs.slice(overflow) : logs,
      dropped_logs: this.state.dropped_logs + overflow,
      queue: { ...this.state.queue, log_depth: Math.min(MAX_LOGS, logs.length) },
    });
  }

  private applySnapshot(snapshot: SnapshotDto): void {
    const link = this.state.link;
    this.commit({
      link: {
        ...link,
        state: LINK_STATES[snapshot.link.state] ?? "disconnected",
        kind: (snapshot.link.kind as AppState["link"]["kind"]) ?? link.kind,
        address: snapshot.link.address ?? link.address,
        mtu: snapshot.link.mtu ?? link.mtu,
        max_payload: snapshot.link.max_payload ?? link.max_payload,
        reconnect_attempt: snapshot.link.reconnect_attempt,
        next_retry_ms: snapshot.link.next_retry_ms,
        last_error: snapshot.link.last_error,
      },
      parser: snapshot.parser,
      transport: {
        rx_bytes: snapshot.transport.rx_bytes,
        tx_bytes: snapshot.transport.tx_bytes,
        errors: snapshot.transport.errors,
        reconnects: snapshot.transport.reconnects,
        last_error: snapshot.transport.last_error,
      },
      check: {
        ...this.state.check,
        recv: snapshot.parser.frames,
      },
      voice: {
        ...this.state.voice,
        enabled: snapshot.voice.enabled,
        segments: snapshot.voice.segments,
        interrupted: snapshot.voice.interrupted,
        failures: snapshot.voice.failures,
        text: snapshot.voice.last_text ?? this.state.voice.text,
        phase: snapshot.voice.enabled ? this.state.voice.phase : "idle",
      },
      asr: {
        ...this.state.asr,
        ready: snapshot.voice.enabled,
        model_path: this.state.asr.model_path || this.state.settings.voice.asr.model_path,
      },
      dropped_logs: this.state.dropped_logs + snapshot.transport.dropped,
    });
  }

  private async refresh(): Promise<void> {
    if (this.disposed) return;
    try {
      this.applySnapshot(await invoke<SnapshotDto>("app_snapshot"));
    } catch (error) {
      this.addLog("warn", "app", `快照刷新失败：${commandError(error)}`);
    }
  }

  // -- 事件 -------------------------------------------------------------

  private onLinkState(payload: LinkStateEvent): void {
    const state = LINK_STATES[payload.state] ?? "disconnected";
    const previous = this.state.link.state;
    this.commit({
      link: {
        ...this.state.link,
        state,
        kind: (payload.kind as AppState["link"]["kind"]) ?? this.state.link.kind,
        address: payload.address ?? this.state.link.address,
        mtu: payload.mtu ?? this.state.link.mtu,
        max_payload: payload.max_payload ?? this.state.link.max_payload,
        reconnect_attempt: payload.reconnect_attempt,
        next_retry_ms: payload.next_retry_ms,
        last_error: payload.last_error,
      },
    });
    if (previous === state) return;
    if (state === "connected") {
      this.addLog("info", "link", `已连接 ${payload.address ?? ""} mtu=${payload.mtu ?? "?"}`);
    } else if (state === "reconnecting") {
      this.addLog(
        "warn",
        "link",
        `链路断开，第 ${payload.reconnect_attempt ?? 1} 次重试（${payload.next_retry_ms ?? 0} ms 后）`,
      );
    } else if (state === "disconnected") {
      this.addLog("info", "link", "链路已断开");
    } else if (state === "error") {
      this.addLog("error", "link", payload.last_error ?? "链路错误");
    }
  }

  private onFrameBatch(payload: FrameBatchEvent): void {
    if (payload.frames.length === 0) return;
    const bytes = payload.frames.reduce((sum, frame) => sum + frame.len + 11, 0);
    const parser = this.state.parser;
    const transport = this.state.transport;
    this.commit({
      parser: { ...parser, frames: parser.frames + payload.frames.length, bytes: parser.bytes + bytes },
      transport: { ...transport, rx_bytes: transport.rx_bytes + bytes },
      check: { ...this.state.check, recv: this.state.check.recv + payload.frames.length },
      queue: { ...this.state.queue, frame_depth: Math.min(payload.frames.length, 200) },
    });
  }

  private onVoiceResult(payload: VoiceResultEvent): void {
    const phase: VoicePhase = payload.note ? "failed" : "done";
    const outcome: InjectOutcome = payload.inject.ok
      ? payload.inject.mode === "clipboard"
        ? "clipboard"
        : payload.inject.mode === "sendinput"
          ? "sendinput"
          : "none"
      : "blocked";
    this.commit({
      voice: {
        ...this.state.voice,
        phase,
        seg_id: payload.seg_id,
        text: payload.text || undefined,
        language: payload.language || undefined,
        audio_ms: payload.audio_ms,
        latency_ms: payload.latency_ms,
        elapsed_ms: payload.audio_ms,
        segments: this.state.voice.segments + (payload.note ? 0 : 1),
        failures: this.state.voice.failures + (payload.note ? 1 : 0),
        queue_depth: 0,
        last_error: payload.note ?? payload.inject.error,
        inject: {
          mode: this.state.voice.inject_mode,
          outcome,
          error: payload.inject.error ?? payload.note,
        },
      },
    });
    if (payload.note) {
      this.addLog("warn", "voice", `段 ${payload.seg_id} 未产出文本：${payload.note}`);
    } else if (!payload.inject.ok) {
      this.addLog(
        "error",
        "voice",
        `识别完成 seg=${payload.seg_id} ${payload.latency_ms}ms「${payload.text}」，但注入失败：${payload.inject.error ?? "未知原因"}`,
      );
    } else {
      this.addLog(
        "info",
        "voice",
        `识别完成 seg=${payload.seg_id} ${payload.latency_ms}ms「${payload.text}」；注入=${payload.inject.mode}`,
      );
    }
  }

  private onAppError(payload: AppErrorEvent): void {
    this.addLog("error", "app", payload.message);
  }

  // -- 动作 -------------------------------------------------------------

  connect = async (): Promise<void> => {
    const kind = this.state.settings.link.kind;
    if (kind !== "ble") {
      this.commit({
        link: { ...this.state.link, state: "error", last_error: "E_NOT_IMPLEMENTED" },
      });
      this.addLog("error", "link", "当前只实现了 BLE，USB 连接待 P2a 真机验证");
      return;
    }
    this.commit({ link: { ...this.state.link, state: "connecting", kind } });
    try {
      await invoke("link_connect", {
        address: null,
        name: this.state.settings.link.ble.name,
        pairedOnly: this.state.settings.link.ble.paired_only,
        scanTimeoutMs: 6000,
      });
      await this.refresh();
    } catch (error) {
      const detail = commandError(error);
      if (detail.startsWith("E_CANCELLED")) {
        this.commit({ link: { ...this.state.link, state: "disconnected", last_error: undefined } });
        this.addLog("info", "link", "已终止连接");
        return;
      }
      this.commit({
        link: { ...this.state.link, state: "error", last_error: detail },
      });
      this.addLog("error", "link", `连接失败：${detail}`);
    }
  };

  cancelConnect = async (): Promise<void> => {
    if (this.state.link.state !== "connecting") return;
    // 物理连接无法立刻中断：先让 UI 回到「已断开」，后台在本次请求结束（≤10 s）后自行清理。
    this.commit({ link: { ...this.state.link, state: "disconnected" } });
    try {
      await invoke("link_cancel_connect");
      this.addLog("warn", "link", "已请求终止连接（后台最多 10 s 内释放）");
    } catch (error) {
      this.addLog("warn", "link", `终止连接失败：${commandError(error)}`);
    }
  };

  disconnect = async (): Promise<void> => {
    try {
      await invoke("link_disconnect");
    } catch (error) {
      this.addLog("warn", "link", `断开失败：${commandError(error)}`);
    }
    await this.refresh();
  };

  retry = async (): Promise<void> => {
    try {
      await invoke("link_retry");
    } catch (error) {
      this.addLog("warn", "link", `重试失败：${commandError(error)}`);
    }
  };

  simulateLinkLoss = async (): Promise<void> => {
    this.addLog("warn", "link", "真实链路不支持模拟断连（仅 Mock 可用）；请直接断开设备");
  };

  stopReconnect = async (): Promise<void> => {
    try {
      await invoke("link_reconnect_policy", { auto: false });
      this.addLog("info", "link", "已停止自动重连");
    } catch (error) {
      this.addLog("warn", "link", `停止重连失败：${commandError(error)}`);
    }
  };

  updateSettings = async (patch: DeepPartial<AppSettings>): Promise<AppSettings> => {
    const settings = mergeDeep(cloneSettings(this.state.settings), patch);
    persistSettings(settings);
    this.commit({ settings });
    if (patch.link?.auto_reconnect !== undefined) {
      await invoke("link_reconnect_policy", { auto: settings.link.auto_reconnect }).catch(
        (error) => this.addLog("warn", "app", `下发重连策略失败：${commandError(error)}`),
      );
    }
    if (patch.voice?.enabled !== undefined) {
      await this.setVoiceEnabled(settings.voice.enabled);
    }
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
    this.commit({ voice: { ...this.state.voice, enabled } });
    try {
      await invoke("voice_set_enabled", {
        enabled,
        modelPath: this.state.settings.voice.asr.model_path || null,
      });
      this.addLog("info", "voice", enabled ? "语音链路已启用（首次会加载模型）" : "语音链路已关闭");
    } catch (error) {
      this.commit({
        voice: { ...this.state.voice, enabled: false, last_error: commandError(error) },
      });
      this.addLog("error", "voice", `语音启用失败：${commandError(error)}`);
    }
    await this.refresh();
  };

  setInjectMode = async (mode: InjectMode): Promise<void> => {
    this.commit({ voice: { ...this.state.voice, inject_mode: mode } });
    this.addLog(
      "warn",
      "voice",
      `注入模式 ${mode} 仅保存在本机；Rust 侧当前固定 auto（SendInput 优先，失败回退剪贴板）`,
    );
  };

  simulatePtt = async (): Promise<void> => {
    this.addLog("info", "voice", "真实链路请直接按设备 PTT（Mock 的模拟按键不可用）");
  };

  cancelVoice = async (): Promise<void> => {
    this.addLog("warn", "voice", "取消识别尚未接入 Rust（串行队列，等当前段结束）");
  };

  retryVoice = async (): Promise<void> => {
    this.addLog("warn", "voice", "重试识别尚未接入 Rust");
  };

  applyKey = async (_keyId: number, config: KeyConfig): Promise<ApplyResult> => {
    const pending = this.state.keys.pending_push + 1;
    this.commit({ keys: { ...this.state.keys, pending_push: pending } });
    return { target: config.route === "host" ? "host" : "both", applied: false, pending };
  };
}
