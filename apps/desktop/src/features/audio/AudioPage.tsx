import { useAppState } from "../../lib/store";
import { backend } from "../../lib/ipc";
import { useI18n } from "../../lib/i18n";
import { useToast } from "../../lib/toast";
import { Banner, Button, Card, EmptyState, Field, PageHeader, StatCard, StatusPill, Switch } from "../../components/primitives";
import { Icon } from "../../components/Icon";
import { VOICE_TONE } from "../../lib/status";
import { formatDuration, formatNumber } from "../../lib/format";
import { SplitHandle, useResizablePanel } from "../../components/ResizablePanel";
import type { InjectMode } from "../../lib/types";

const injectModes: Array<{ value: InjectMode; labelKey: string }> = [
  { value: "auto", labelKey: "audio.voice.injectAuto" },
  { value: "sendinput", labelKey: "audio.voice.injectSendinput" },
  { value: "clipboard", labelKey: "audio.voice.injectClipboard" },
  { value: "recognize_only", labelKey: "audio.voice.injectRecognizeOnly" },
];

const phaseOrder = ["recording", "transcribing", "injecting", "done"] as const;

export function AudioPage() {
  const state = useAppState();
  const { t, locale } = useI18n();
  const { push } = useToast();
  const voice = state.voice;
  const sidePanel = useResizablePanel({
    storageKey: "audioSide",
    initial: 320,
    min: 260,
    max: 440,
    edge: "left",
    label: t("layout.resizeAudioSide"),
  });
  const busy = voice.phase === "recording" || voice.phase === "finalizing" || voice.phase === "transcribing" || voice.phase === "injecting";
  const canSimulate = voice.enabled && !busy;

  const errorCode = voice.last_error ?? "E_ASR_FAILED";
  const errorLabel = t(`errors.${errorCode}`) === `⟦errors.${errorCode}⟧` ? errorCode : t(`errors.${errorCode}`);

  const copy = async (value: string, level: "success" | "danger" = "success") => {
    try {
      await navigator.clipboard.writeText(value);
      push({ level, titleKey: "common.copied" });
    } catch {
      push({ level: "danger", titleKey: "common.copy" });
    }
  };

  const currentPhase = voice.phase === "finalizing" ? "transcribing" : voice.phase;
  const phaseIndex = phaseOrder.indexOf(currentPhase as (typeof phaseOrder)[number]);

  return (
    <div className="page">
      <PageHeader
        title={t("audio.title")}
        subtitle={t("audio.subtitle")}
        actions={
          <StatusPill
            tone={VOICE_TONE[voice.phase]}
            label={`${t("audio.voice.eyebrow")} · ${t(`audio.voice.state.${voice.phase}`)}`}
          />
        }
      />

      <div className="page-body">
        <div className="audio-layout" style={sidePanel.style}>
          <Card
            className="voice-card"
            title={
              <span className="row" style={{ gap: 8 }}>
                <span className="voice-flag" aria-hidden="true" />
                {t("audio.voice.title")}
              </span>
            }
            subtitle={t("audio.voice.eyebrow")}
            actions={
              <div className="row">
                {voice.enabled ? (
                  <>
                    <Button variant="secondary" size="sm" onClick={() => void backend.setVoiceEnabled(false)}>
                      {t("audio.voice.disable")}
                    </Button>
                    <Button
                      variant="primary"
                      size="sm"
                      icon="play"
                      disabled={!canSimulate}
                      onClick={() => void backend.simulatePtt()}
                    >
                      {t("audio.voice.simulatePtt")}
                    </Button>
                  </>
                ) : (
                  <Button variant="primary" icon="power" onClick={() => void backend.setVoiceEnabled(true)}>
                    {t("audio.voice.enable")}
                  </Button>
                )}
              </div>
            }
          >
            <div className="voice-body">
              <div className="voice-pipeline" aria-label={t("audio.voice.title")}>
                {phaseOrder.map((phase, index) => (
                  <span
                    key={phase}
                    className={`voice-step${index === phaseIndex ? " is-active" : ""}${index < phaseIndex ? " is-done" : ""}`}
                  >
                    <span className="voice-step__index tabular">{index + 1}</span>
                    {t(`audio.voice.state.${phase}`)}
                  </span>
                ))}
              </div>

              {voice.phase === "idle" ? (
                <EmptyState icon="wave" title={t("audio.voice.idleTitle")} text={t("audio.voice.idleText")} />
              ) : null}

              {voice.phase === "recording" || voice.phase === "finalizing" ? (
                <div className="voice-recording">
                  <div className="waveform" aria-hidden="true">
                    {Array.from({ length: 32 }, (_, index) => (
                      <span
                        key={index}
                        className="wave-bar"
                        style={{ height: `${18 + ((index * 13) % 46)}%`, animationDelay: `${index * 38}ms` }}
                      />
                    ))}
                  </div>
                  <div className="voice-recording__meta">
                    <div className="stack" style={{ gap: 2 }}>
                      <span className="voice-recording__title">
                        {voice.phase === "recording" ? t("audio.voice.recordingTitle") : t("audio.voice.finalizingText")}
                      </span>
                      <span className="muted tabular">
                        {t("audio.voice.recordingText", {
                          seg: voice.seg_id ?? "—",
                          blocks: Math.floor((voice.elapsed_ms ?? 0) / 20),
                        })}
                      </span>
                    </div>
                    <span className="voice-timer tabular">{formatDuration(voice.elapsed_ms ?? 0)}</span>
                  </div>
                  <Button variant="secondary" icon="x" onClick={() => void backend.cancelVoice()}>
                    {t("audio.voice.cancel")}
                  </Button>
                </div>
              ) : null}

              {voice.phase === "transcribing" || voice.phase === "queued" || voice.phase === "injecting" ? (
                <div className="voice-transcribing">
                  <div className="stack" style={{ gap: 10 }}>
                    <span className="voice-recording__title">
                      {voice.phase === "injecting" ? t("audio.voice.injectingText") : t("audio.voice.transcribingText")}
                    </span>
                    <div className="skeleton" style={{ height: 14, width: "82%" }} />
                    <div className="skeleton" style={{ height: 14, width: "64%" }} />
                    <div className="row row--between">
                      <span className="muted tabular">{formatDuration(voice.elapsed_ms ?? 0)}</span>
                      <Button variant="ghost" size="sm" onClick={() => void backend.cancelVoice()}>
                        {t("audio.voice.cancel")}
                      </Button>
                    </div>
                  </div>
                </div>
              ) : null}

              {voice.phase === "done" ? (
                <div className="voice-result">
                  <div className="row row--between row--wrap">
                    <span className="eyebrow">{t("audio.voice.resultTitle")}</span>
                    <span className="row" style={{ gap: 9 }}>
                      <Button variant="ghost" size="sm" icon="copy" onClick={() => void copy(voice.text ?? "")}>
                        {t("audio.voice.copyText")}
                      </Button>
                    </span>
                  </div>
                  <p className="voice-text" data-testid="voice-text">{voice.text}</p>
                  <div className="voice-meta">
                    <span>{t("audio.voice.language")} <strong>{voice.language ?? "—"}</strong></span>
                    <span>{t("audio.voice.audioDuration")} <strong className="tabular">{formatDuration(voice.audio_ms ?? 0)}</strong></span>
                    <span>{t("audio.voice.latency")} <strong className="tabular">{formatDuration(voice.latency_ms ?? 0)}</strong></span>
                  </div>
                  <div className={`inject-row inject-row--${voice.inject?.outcome ?? "none"}`}>
                    <Icon
                      name={voice.inject?.outcome === "blocked" ? "alertTriangle" : voice.inject?.outcome === "none" ? "info" : "check"}
                      size={15}
                    />
                    <span>{t(`audio.inject.${voice.inject?.outcome ?? "none"}`)}</span>
                    {voice.inject?.outcome === "blocked" ? <span className="muted">{t("audio.inject.uipiHint")}</span> : null}
                  </div>
                  {voice.inject?.outcome === "blocked" && (voice.inject.error ?? voice.last_error) ? (
                    <p className="field__hint" data-testid="inject-error">{voice.inject.error ?? voice.last_error}</p>
                  ) : null}
                </div>
              ) : null}

              {voice.phase === "failed" ? (
                <div className="stack">
                  <Banner
                    tone="danger"
                    title={t("audio.voice.failedTitle")}
                    actions={
                      <Button variant="secondary" icon="refresh" onClick={() => void backend.retryVoice()}>
                        {t("audio.voice.retry")}
                      </Button>
                    }
                  >
                    {errorLabel} · {t("audio.voice.failureRetained")}
                  </Banner>
                </div>
              ) : null}
            </div>

            <div className="voice-options">
              <Field label={t("audio.voice.injectMode")} htmlFor="inject-mode">
                <select
                  id="inject-mode"
                  className="select"
                  value={voice.inject_mode}
                  disabled={busy}
                  onChange={(event) => void backend.setInjectMode(event.target.value as InjectMode)}
                >
                  {injectModes.map((mode) => (
                    <option key={mode.value} value={mode.value}>{t(mode.labelKey)}</option>
                  ))}
                </select>
              </Field>
              <p className="field__hint">{t("audio.inject.focusHint")}</p>
            </div>
          </Card>

          <div className="resizable-panel audio-side-panel">
          <div className="audio-side">
            <Card title={t("audio.stats.title")} bodyClassName="audio-stats">
              <StatCard label={t("audio.stats.segments")} value={formatNumber(locale, voice.segments)} note={t("audio.voice.segments")} />
              <StatCard label={t("audio.stats.failures")} value={formatNumber(locale, voice.failures)} note={t("audio.voice.state.failed")} alert={voice.failures > 0} />
              <StatCard label={t("audio.stats.interrupted")} value={formatNumber(locale, voice.interrupted)} note={t("audio.voice.state.idle")} />
              <StatCard label={t("audio.stats.queue")} value={formatNumber(locale, voice.queue_depth)} note={t("audio.voice.queue")} />
            </Card>

            <Card title={t("audio.playback.title")}>
              <div className="stack">
                <Switch
                  id="audio-receive"
                  checked={state.settings.audio.receive}
                  onChange={(checked) => void backend.updateSettings({ audio: { receive: checked } })}
                  label={t("audio.playback.receive")}
                />
                <Switch
                  id="audio-playback"
                  checked={state.settings.audio.playback}
                  onChange={(checked) => void backend.updateSettings({ audio: { playback: checked } })}
                  label={t("audio.playback.playback")}
                />
                <Field label={t("audio.playback.buffer")} htmlFor="audio-buffer">
                  <div className="input-group">
                    <input
                      id="audio-buffer"
                      className="input tabular"
                      type="number"
                      min={10}
                      max={60}
                      value={state.settings.audio.buf_ms}
                      onChange={(event) => void backend.updateSettings({ audio: { buf_ms: Number(event.target.value) } })}
                    />
                    <span className="input-group__suffix">ms</span>
                  </div>
                </Field>
                <p className="field__hint">{t("audio.playback.noWav")}</p>
              </div>
            </Card>
          </div>
            <SplitHandle edge="left" handleProps={sidePanel.handleProps} />
          </div>
        </div>
      </div>
    </div>
  );
}
