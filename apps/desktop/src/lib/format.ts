import type { Locale } from "./i18n-core";

const numberCache = new Map<string, Intl.NumberFormat>();

function numberFormatter(locale: Locale, options?: Intl.NumberFormatOptions): Intl.NumberFormat {
  const key = `${locale}:${JSON.stringify(options ?? {})}`;
  const cached = numberCache.get(key);
  if (cached) return cached;
  const formatter = new Intl.NumberFormat(locale, options);
  numberCache.set(key, formatter);
  return formatter;
}

export function formatNumber(locale: Locale, value: number, maximumFractionDigits = 0): string {
  if (!Number.isFinite(value)) return "—";
  return numberFormatter(locale, { maximumFractionDigits }).format(value);
}

export function formatBytes(locale: Locale, bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "—";
  if (bytes < 1024) return `${formatNumber(locale, bytes)} B`;
  if (bytes < 1024 * 1024) return `${formatNumber(locale, bytes / 1024, 1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${formatNumber(locale, bytes / 1024 / 1024, 1)} MB`;
  return `${formatNumber(locale, bytes / 1024 / 1024 / 1024, 2)} GB`;
}

export function formatRate(locale: Locale, bytesPerSecond: number): string {
  if (!Number.isFinite(bytesPerSecond) || bytesPerSecond <= 0) return "0 B/s";
  if (bytesPerSecond < 1024) return `${formatNumber(locale, bytesPerSecond)} B/s`;
  if (bytesPerSecond < 1024 * 1024) return `${formatNumber(locale, bytesPerSecond / 1024, 1)} KB/s`;
  return `${formatNumber(locale, bytesPerSecond / 1024 / 1024, 2)} MB/s`;
}

export function formatDuration(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) return "—";
  if (ms < 1000) return `${Math.round(ms)} ms`;
  return `${(ms / 1000).toFixed(ms < 10_000 ? 2 : 1)} s`;
}

export function formatClock(locale: Locale, atMs: number): string {
  return new Intl.DateTimeFormat(locale, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  }).format(new Date(atMs));
}

export function formatLogTime(locale: Locale, atMs: number): string {
  return new Intl.DateTimeFormat(locale, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    fractionalSecondDigits: 3,
    hour12: false,
  }).format(new Date(atMs));
}

export function formatDateTime(locale: Locale, atMs: number): string {
  return new Intl.DateTimeFormat(locale, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  }).format(new Date(atMs));
}

export function formatUptime(totalSeconds: number): { h: number; m: number; s: number } {
  const safe = Math.max(0, Math.floor(totalSeconds));
  return {
    h: Math.floor(safe / 3600),
    m: Math.floor((safe % 3600) / 60),
    s: safe % 60,
  };
}

export function pad2(value: number): string {
  return String(value).padStart(2, "0");
}
