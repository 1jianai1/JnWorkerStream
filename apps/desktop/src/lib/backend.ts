import type { AppSettings, AppState, DeepPartial, InjectMode, KeyConfig } from "./types";

export interface ConfigIssue {
  corrupt: boolean;
  backup?: string;
}

export interface ApplyResult {
  target: "device" | "host" | "both";
  applied: boolean;
  pending: number;
}

/**
 * The only surface components may use to reach the backend.
 * P1 implements it with MockBackend; P2a adds a Tauri-backed implementation.
 */
export interface Backend {
  subscribe(listener: () => void): () => void;
  getState(): AppState;
  getConfigIssue(): ConfigIssue;
  connect(): Promise<void>;
  /** 连接中（`link.state === "connecting"`）时终止本次连接。 */
  cancelConnect(): Promise<void>;
  disconnect(): Promise<void>;
  retry(): Promise<void>;
  simulateLinkLoss(): Promise<void>;
  stopReconnect(): Promise<void>;
  updateSettings(patch: DeepPartial<AppSettings>): Promise<AppSettings>;
  resetSettings(): Promise<AppSettings>;
  clearLogs(): Promise<void>;
  setVoiceEnabled(enabled: boolean): Promise<void>;
  setInjectMode(mode: InjectMode): Promise<void>;
  simulatePtt(): Promise<void>;
  cancelVoice(): Promise<void>;
  retryVoice(): Promise<void>;
  applyKey(keyId: number, config: KeyConfig): Promise<ApplyResult>;
}
