import { useAppState } from "../../lib/store";
import { backend } from "../../lib/ipc";
import { useI18n } from "../../lib/i18n";
import { useToast } from "../../lib/toast";
import { Banner, Button, Card, EmptyState, Field, PageHeader, Progress, Segmented, StatCard, Switch } from "../../components/primitives";
import { LogMessage } from "../../components/LogMessage";
import { SplitHandle, useResizablePanel } from "../../components/ResizablePanel";
import { LinkStatePill } from "../../app/LinkStatePill";
import { formatBytes, formatDuration, formatLogTime, formatNumber, formatRate, formatUptime } from "../../lib/format";
import { LOG_TONE } from "../../lib/status";
import type { LogEntry, TransportKind } from "../../lib/types";

const transportOptions = [
  { value: "usb" as const, labelKey: "link.control.usb" },
  { value: "ble" as const, labelKey: "link.control.ble" },
];

function uptimeText(openedAt: number | undefined, locale: "zh-CN" | "en-US"): string {
  if (!openedAt) return "—";
  const parts = formatUptime((Date.now() - openedAt) / 1000);
  return `${formatNumber(locale, parts.h)}h ${String(parts.m).padStart(2, "0")}m ${String(parts.s).padStart(2, "0")}s`;
}

export function LinkPage() {
  const state = useAppState();
  const { t, locale } = useI18n();
  const { push } = useToast();
  const { link } = state;
  const connected = link.state === "connected";
  const connecting = link.state === "connecting";
  const error = link.state === "error";
  const reconnecting = link.state === "reconnecting";
  const kind = state.settings.link.kind;
  const controlPanel = useResizablePanel({
    storageKey: "linkControl",
    initial: 320,
    min: 260,
    max: 440,
    edge: "right",
    label: t("layout.resizeLinkControl"),
  });
  const queuePercent = Math.round((state.queue.frame_depth / state.queue.frame_capacity) * 100);
  const logPercent = Math.round((state.queue.log_depth / state.queue.log_capacity) * 100);
  const recent = [...state.logs].slice(-8).reverse();

  const changeKind = async (next: TransportKind) => {
    await backend.updateSettings({ link: { kind: next } });
  };

  const connect = async () => {
    await backend.connect();
    push({ level: "info", titleKey: "link.control.connecting" });
  };

  const errorCode = link.last_error ?? "E_WRITE_FAILED";
  const errorLabel = t(`errors.${errorCode}`) === `⟦errors.${errorCode}⟧` ? errorCode : t(`errors.${errorCode}`);

  return (
    <div className="page">
      <PageHeader
        title={t("link.title")}
        subtitle={t("link.subtitle")}
      />

      <div className="page-body">
        <div className="link-layout" style={controlPanel.style}>
          <div className="resizable-panel">
            <Card title={t("link.control.title")} className="link-control">
            <div className="stack">
              <Field label={t("link.control.transport")}>
                <Segmented
                  ariaLabel={t("link.control.transport")}
                  value={kind}
                  options={transportOptions.map((option) => ({
                    value: option.value,
                    label: t(option.labelKey),
                    disabled: connected || connecting,
                  }))}
                  onChange={(next) => void changeKind(next)}
                />
              </Field>

              {kind === "usb" ? (
                <>
                  <Field label={t("link.control.usbDevice")} htmlFor="usb-device">
                    <select
                      id="usb-device"
                      className="select"
                      value={state.settings.link.usb.device}
                      disabled={connected || connecting}
                      onChange={(event) =>
                        void backend.updateSettings({ link: { usb: { device: event.target.value } } })
                      }
                    >
                      <option value="USB CDC · COM3">{t("mock.usbDevice.dev1")}</option>
                      <option value="USB CDC · COM4">{t("mock.usbDevice.dev2")}</option>
                      <option value="USB CDC · COM5">{t("mock.usbDevice.dev3")}</option>
                    </select>
                  </Field>
                </>
              ) : null}

              {kind === "ble" ? (
                <>
                  <Field label={t("link.control.deviceName")} htmlFor="ble-name">
                    <input id="ble-name" className="input" value={state.settings.link.ble.name} readOnly />
                  </Field>
                  <Switch
                    id="paired-only"
                    checked={state.settings.link.ble.paired_only}
                    onChange={(checked) => void backend.updateSettings({ link: { ble: { paired_only: checked } } })}
                    label={t("link.control.pairedOnly")}
                  />
                  <p className="field__hint">{t("link.control.hintBle")}</p>
                </>
              ) : null}

              <Switch
                id="auto-reconnect"
                checked={state.settings.link.auto_reconnect}
                onChange={(checked) => void backend.updateSettings({ link: { auto_reconnect: checked } })}
                label={t("link.control.autoReconnect")}
              />

              <div className="stack stack--tight">
                {connected ? (
                  <Button variant="secondary" icon="power" block onClick={() => void backend.disconnect()}>
                    {t("link.control.disconnect")}
                  </Button>
                ) : connecting ? (
                  <Button variant="secondary" icon="x" block onClick={() => void backend.cancelConnect()}>
                    {t("link.control.cancelConnect")}
                  </Button>
                ) : (
                  <Button
                    variant="primary"
                    icon={reconnecting ? "refresh" : "link"}
                    block
                    onClick={() => void (reconnecting ? backend.retry() : connect())}
                  >
                    {reconnecting ? t("link.banner.retryNow") : t("link.control.connect")}
                  </Button>
                )}
                {reconnecting ? (
                  <Button variant="ghost" size="sm" block onClick={() => void backend.stopReconnect()}>
                    {t("link.banner.stop")}
                  </Button>
                ) : null}
              </div>

              <p className="field__hint">
                {kind === "ble" ? t("link.control.hintBle") : t("link.control.hintUsb")}
              </p>
            </div>
          </Card>
            <SplitHandle edge="right" handleProps={controlPanel.handleProps} />
          </div>

          <div className="dash-main">
            {error ? (
              <Banner
                tone="danger"
                title={t("link.banner.errorTitle")}
                actions={
                  <Button variant="secondary" icon="refresh" onClick={() => void backend.retry()}>
                    {t("common.retry")}
                  </Button>
                }
              >
                {t("link.banner.errorDetail", { code: errorLabel, detail: link.last_error_detail ?? "" })} · {t("link.banner.nextStep")}
              </Banner>
            ) : null}

            {reconnecting ? (
              <Banner
                tone="warning"
                title={t("link.banner.reconnectingTitle")}
                actions={
                  <>
                    <Button variant="secondary" size="sm" onClick={() => void backend.retry()}>
                      {t("link.banner.retryNow")}
                    </Button>
                    <Button variant="ghost" size="sm" onClick={() => void backend.stopReconnect()}>
                      {t("link.banner.stop")}
                    </Button>
                  </>
                }
              >
                {t("link.banner.reconnectingText", {
                  attempt: link.reconnect_attempt ?? 1,
                  seconds: Math.round((link.next_retry_ms ?? 2000) / 1000),
                })}
              </Banner>
            ) : null}

            <div className="stat-grid">
              <StatCard
                label={t("link.stats.frames")}
                value={formatNumber(locale, state.parser.frames)}
                note={t("link.stats.framesNote")}
                testId="stat-frames"
              />
              <StatCard
                label={t("link.stats.throughput")}
                value={formatRate(locale, state.check.throughput_bps)}
                note={t("link.stats.throughputNote")}
              />
              <StatCard
                label={t("link.stats.crc")}
                value={formatNumber(locale, state.parser.crc_err)}
                note={t("link.stats.crcNote")}
                alert={state.parser.crc_err > 0}
              />
              <StatCard
                className="stat-card--wide-only"
                label={t("link.stats.lost")}
                value={formatNumber(locale, state.check.lost)}
                note={t("link.stats.lostNote")}
                alert={state.check.lost > 0}
              />
            </div>

            <div className="dash-grid">
              <Card title={t("link.detail.title")} className="link-detail">
                {link.state === "disconnected" ? (
                  <EmptyState icon="plug" title={t("link.detail.empty")} text={t("link.control.hintDisconnected")} />
                ) : (
                  <dl className="detail-list">
                    <div>
                      <dt>{t("link.detail.state")}</dt>
                      <dd><LinkStatePill state={link.state} /></dd>
                    </div>
                    <div>
                      <dt>{t("link.detail.kind")}</dt>
                      <dd>{link.kind ? t(`link.control.${link.kind}`) : "—"}</dd>
                    </div>
                    <div>
                      <dt>{link.kind === "usb" ? t("link.detail.usbDevice") : t("link.detail.address")}</dt>
                      <dd className="mono">{link.address ?? "—"}</dd>
                    </div>
                    <div>
                      <dt>{t("link.detail.mtu")}</dt>
                      <dd className="tabular">{link.mtu ?? "—"}</dd>
                    </div>
                    <div>
                      <dt>{t("link.detail.maxPayload")}</dt>
                      <dd className="tabular">{link.max_payload ?? "—"} B</dd>
                    </div>
                    <div>
                      <dt>{t("link.detail.uptime")}</dt>
                      <dd className="tabular">{uptimeText(link.opened_at_ms, locale)}</dd>
                    </div>
                    <div>
                      <dt>{t("link.stats.lost")}</dt>
                      <dd className="tabular">{formatNumber(locale, state.check.lost)}</dd>
                    </div>
                    <div>
                      <dt>{t("link.stats.jitter")}</dt>
                      <dd className="tabular">{state.check.jitter_ms.toFixed(2)} ms</dd>
                    </div>
                  </dl>
                )}
              </Card>

              <Card
                title={t("link.recent.title")}
                actions={
                  <span className="row" style={{ gap: 6 }}>
                    <span className="muted" style={{ font: "var(--jn-font-caption)" }}>{formatBytes(locale, state.transport.rx_bytes)}</span>
                  </span>
                }
                flush
              >
                {recent.length === 0 ? (
                  <EmptyState icon="terminal" title={t("link.recent.empty")} />
                ) : (
                  <ul className="event-list">
                    {recent.map((entry: LogEntry, index) => (
                      <li key={`${entry.at_ms}-${index}`} className="event-row">
                        <span className="event-time tabular">{formatLogTime(locale, entry.at_ms)}</span>
                        <span className={`log-dot log-dot--${LOG_TONE[entry.level]}`} aria-hidden="true" />
                        <span className="event-domain">{t(`logs.domain.${entry.domain}`)}</span>
                        <span className="event-text mono" title={entry.textKey ? t(entry.textKey, entry.params) : entry.text}>
                          <LogMessage entry={entry} />
                        </span>
                      </li>
                    ))}
                  </ul>
                )}
              </Card>
            </div>

            <Card title={t("link.capacity.title")}>
              <div className="stack">
                <div className="meter-row">
                  <span className="meter-label">{t("link.capacity.queue")}</span>
                  <Progress value={queuePercent} tone={queuePercent > 80 ? "warning" : "info"} />
                  <span className="meter-value tabular">{queuePercent}%</span>
                </div>
                <div className="meter-row">
                  <span className="meter-label">{t("link.capacity.logs")}</span>
                  <Progress value={logPercent} tone={logPercent > 80 ? "warning" : "info"} />
                  <span className="meter-value tabular">{logPercent}%</span>
                </div>
                <hr className="divider" />
                <div className="row row--between row--wrap">
                  <span className="field__hint">{t("link.capacity.hint")}</span>
                  <span className="row" style={{ gap: 16 }}>
                    <span className="muted">{t("link.stats.rate")} <strong className="tabular">{formatNumber(locale, state.check.rate_fps)}</strong> fps</span>
                    <span className="muted">{t("link.stats.jitter")} <strong className="tabular">{formatDuration(state.check.mean_interval_ms)}</strong></span>
                  </span>
                </div>
              </div>
            </Card>
          </div>
        </div>
      </div>
    </div>
  );
}
