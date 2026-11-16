import type { KeyRoute, KeyTrigger, LinkState, LogLevel, VoicePhase } from "./types";

export type Tone = "neutral" | "success" | "warning" | "danger" | "info" | "voice";

export const LINK_TONE: Record<LinkState, Tone> = {
  disconnected: "neutral",
  connecting: "warning",
  connected: "success",
  reconnecting: "warning",
  error: "danger",
};

export const LOG_TONE: Record<LogLevel, Tone> = {
  trace: "neutral",
  debug: "neutral",
  info: "info",
  warn: "warning",
  error: "danger",
};

export const VOICE_TONE: Record<VoicePhase, Tone> = {
  idle: "neutral",
  recording: "voice",
  finalizing: "voice",
  queued: "info",
  transcribing: "info",
  injecting: "voice",
  done: "success",
  failed: "danger",
};

export const ROUTE_TONE: Record<KeyRoute, Tone> = {
  devicePassthrough: "neutral",
  deviceMacro: "voice",
  host: "info",
};

export const TRIGGERS: KeyTrigger[] = ["tap", "double", "long", "repeat"];
export const ROUTES: KeyRoute[] = ["devicePassthrough", "deviceMacro", "host"];

export function isDeviceRoute(route: KeyRoute): boolean {
  return route !== "host";
}

export function triggerAvailable(route: KeyRoute): boolean {
  return route === "host";
}
