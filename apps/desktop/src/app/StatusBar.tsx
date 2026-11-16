import type { AppState } from "../lib/types";
import { useI18n } from "../lib/i18n";
import { formatBytes, formatNumber } from "../lib/format";

export function StatusBar({ state }: { state: AppState }) {
  const { t, locale } = useI18n();
  const queuePercent = state.queue.frame_capacity > 0
    ? Math.round((state.queue.frame_depth / state.queue.frame_capacity) * 100)
    : 0;
  const configLabel = state.keys.pending_push > 0
    ? t("status.config.pending")
    : state.keys.last_applied_at_ms
      ? t("status.config.applied")
      : t("status.config.local");
  return (
    <footer className="statusbar">
      <span className="statusbar-item">
        {t("status.rx")} <strong>{formatBytes(locale, state.transport.rx_bytes)}</strong>
      </span>
      <span className="statusbar-sep" aria-hidden="true" />
      <span className="statusbar-item">
        {t("status.tx")} <strong>{formatBytes(locale, state.transport.tx_bytes)}</strong>
      </span>
      <span className="statusbar-sep" aria-hidden="true" />
      <span className="statusbar-item">
        {t("status.dropped")} <strong>{formatNumber(locale, state.dropped_logs)}</strong>
      </span>
      <span className="statusbar-sep" aria-hidden="true" />
      <span className="statusbar-item">
        {t("status.reconnects")} <strong>{formatNumber(locale, state.transport.reconnects)}</strong>
      </span>
      <span className="statusbar-sep" aria-hidden="true" />
      <span className={`statusbar-item${queuePercent > 80 ? " is-warning" : ""}`}>
        {t("status.queue")} <strong>{queuePercent}%</strong>
      </span>
      <span className="statusbar-sep" aria-hidden="true" />
      <span className={`statusbar-item${state.keys.pending_push > 0 ? " is-warning" : ""}`}>
        <strong>{configLabel}</strong>
      </span>
      <span className="statusbar-spacer" />
      <span className="statusbar-item">P1 Mock</span>
      <span className="statusbar-sep" aria-hidden="true" />
      <span className="statusbar-item tabular">v{state.app.version}</span>
    </footer>
  );
}
