import { useMemo, useState } from "react";
import { useAppState } from "../../lib/store";
import { backend } from "../../lib/ipc";
import { useI18n } from "../../lib/i18n";
import { useToast } from "../../lib/toast";
import { Banner, Button, Card, Field, PageHeader, Segmented, StatusPill, Switch } from "../../components/primitives";
import type { AppSettings, DeepPartial, LogLevel } from "../../lib/types";
import type { Locale } from "../../lib/i18n-core";

const levels: LogLevel[] = ["trace", "debug", "info", "warn", "error"];

export function SettingsPage() {
  const state = useAppState();
  const { t, locale, setLocale } = useI18n();
  const { push } = useToast();
  const issue = backend.getConfigIssue();
  const [draft, setDraft] = useState<AppSettings>(state.settings);
  const [saving, setSaving] = useState(false);
  const [showReset, setShowReset] = useState(false);

  const dirty = useMemo(() => JSON.stringify(draft) !== JSON.stringify(state.settings), [draft, state.settings]);

  const patch = (next: DeepPartial<AppSettings>) => {
    setDraft((current) => {
      const merged = JSON.parse(JSON.stringify(current)) as AppSettings;
      const merge = (target: Record<string, unknown>, source: Record<string, unknown>) => {
        for (const [key, value] of Object.entries(source)) {
          if (value && typeof value === "object" && !Array.isArray(value)) {
            const child = (target[key] as Record<string, unknown>) ?? {};
            target[key] = child;
            merge(child, value as Record<string, unknown>);
          } else {
            target[key] = value;
          }
        }
      };
      merge(merged as unknown as Record<string, unknown>, next as Record<string, unknown>);
      return merged;
    });
  };

  const changeLocale = (next: Locale) => {
    setLocale(next);
    setDraft((current) => ({ ...current, ui: { ...current.ui, locale: next } }));
  };

  const save = async () => {
    setSaving(true);
    try {
      const next = { ...draft, ui: { ...draft.ui, locale } };
      const saved = await backend.updateSettings(next);
      setDraft(saved);
      push({ level: "success", titleKey: "settings.saved" });
    } finally {
      setSaving(false);
    }
  };

  const reset = async () => {
    setShowReset(false);
    const next = await backend.resetSettings();
    setDraft(next);
    push({ level: "info", titleKey: "settings.resetDone" });
  };

  return (
    <div className="page">
      <PageHeader
        title={t("settings.title")}
        subtitle={t("settings.subtitle")}
        actions={
          <>
            <span className="tag">{dirty ? t("settings.dirty") : t("settings.clean")}</span>
            <Button variant="ghost" icon="rotate" onClick={() => setShowReset(true)}>
              {t("settings.reset")}
            </Button>
            <Button variant="primary" icon="check" loading={saving} disabled={!dirty} onClick={() => void save()}>
              {t("settings.save")}
            </Button>
          </>
        }
      />

      <div className="page-body">
        {issue.corrupt ? (
          <Banner
            tone="warning"
            title={t("settings.corrupt.title")}
            actions={
              <Button variant="secondary" size="sm" onClick={() => void reset()}>
                {t("settings.corrupt.restore")}
              </Button>
            }
          >
            {t("settings.corrupt.text", { ts: issue.backup ?? "" })}
          </Banner>
        ) : null}

        <div className="settings-grid">
          <Card title={t("settings.ui.title")}>
            <div className="stack">
              <Field label={t("settings.ui.language")} hint={t("settings.ui.languageHint")}>
                <Segmented
                  ariaLabel={t("settings.ui.language")}
                  value={locale}
                  options={[
                    { value: "zh-CN", label: t("topbar.localeZh") + " · " + t("settings.ui.languageNameZh") },
                    { value: "en-US", label: t("topbar.localeEn") + " · " + t("settings.ui.languageNameEn") },
                  ]}
                  onChange={(next) => changeLocale(next)}
                />
              </Field>
              <Field label={t("settings.ui.renderer")} hint={t("settings.ui.rendererHint")}>
                <Segmented
                  ariaLabel={t("settings.ui.renderer")}
                  value={draft.ui.keypad.renderer}
                  options={[
                    { value: "auto", label: t("settings.ui.rendererAuto") },
                    { value: "3d", label: t("settings.ui.renderer3d") },
                    { value: "2d", label: t("settings.ui.renderer2d") },
                  ]}
                  onChange={(next) => patch({ ui: { keypad: { renderer: next } } })}
                />
              </Field>
              <div className="row row--between">
                <span className="field__label">{t("settings.about.version")}</span>
                <span className="mono">{state.app.version}</span>
              </div>
            </div>
          </Card>

          <Card title={t("settings.link.title")}>
            <div className="stack">
              <Field label={t("settings.link.transport")} htmlFor="default-transport">
                <select
                  id="default-transport"
                  className="select"
                  value={draft.link.kind}
                  onChange={(event) => patch({ link: { kind: event.target.value as AppSettings["link"]["kind"] } })}
                >
                  <option value="usb">{t("link.control.usb")}</option>
                  <option value="ble">{t("link.control.ble")}</option>
                </select>
              </Field>
              <Switch
                id="settings-auto-reconnect"
                checked={draft.link.auto_reconnect}
                onChange={(checked) => patch({ link: { auto_reconnect: checked } })}
                label={t("settings.link.autoReconnect")}
              />
              <p className="field__hint">{t("settings.link.autoReconnectHint")}</p>
            </div>
          </Card>

          <Card title={t("settings.logging.title")}>
            <div className="stack">
              <Field label={t("settings.logging.level")} htmlFor="log-level">
                <select
                  id="log-level"
                  className="select"
                  value={draft.logging.level_min}
                  onChange={(event) => patch({ logging: { level_min: event.target.value as LogLevel } })}
                >
                  {levels.map((level) => (
                    <option key={level} value={level}>{t(`logs.level.${level}`)}</option>
                  ))}
                </select>
              </Field>
              <Field label={t("settings.logging.maxEntries")} hint={t("settings.logging.maxEntriesHint")} htmlFor="log-max">
                <input
                  id="log-max"
                  className="input tabular"
                  type="number"
                  min={100}
                  max={20000}
                  step={100}
                  value={draft.logging.max_entries}
                  onChange={(event) => patch({ logging: { max_entries: Number(event.target.value) } })}
                />
              </Field>
            </div>
          </Card>

          <Card title={t("settings.audio.title")}>
            <div className="stack">
              <Field label={t("settings.audio.saveDir")} htmlFor="settings-audio-dir">
                <input
                  id="settings-audio-dir"
                  className="input mono"
                  value={draft.audio.save_dir}
                  onChange={(event) => patch({ audio: { save_dir: event.target.value } })}
                />
              </Field>
              <Switch
                id="settings-receive"
                checked={draft.audio.receive}
                onChange={(checked) => patch({ audio: { receive: checked } })}
                label={t("audio.playback.receive")}
              />
              <Switch
                id="settings-playback"
                checked={draft.audio.playback}
                onChange={(checked) => patch({ audio: { playback: checked } })}
                label={t("audio.playback.playback")}
              />
              <Field label={t("audio.playback.buffer")} htmlFor="settings-buffer">
                <div className="input-group">
                  <input
                    id="settings-buffer"
                    className="input tabular"
                    type="number"
                    min={10}
                    max={60}
                    value={draft.audio.buf_ms}
                    onChange={(event) => patch({ audio: { buf_ms: Number(event.target.value) } })}
                  />
                  <span className="input-group__suffix">ms</span>
                </div>
              </Field>
            </div>
          </Card>

          <Card
            title={t("settings.asr.title")}
            actions={
              <StatusPill
                tone={state.asr.ready ? "success" : "warning"}
                label={state.asr.ready ? t("settings.asr.modelReady") : t("settings.asr.modelMissing")}
              />
            }
          >
            <div className="stack">
              <Field label={t("settings.asr.modelPath")} htmlFor="asr-model">
                <input
                  id="asr-model"
                  className="input mono"
                  value={draft.voice.asr.model_path}
                  onChange={(event) => patch({ voice: { asr: { model_path: event.target.value } } })}
                />
              </Field>
              <div className="grid-2">
                <Field label={t("settings.asr.device")} htmlFor="asr-device">
                  <select
                    id="asr-device"
                    className="select"
                    value={draft.voice.asr.device}
                    onChange={(event) => patch({ voice: { asr: { device: event.target.value as "cpu" | "cuda" } } })}
                  >
                    <option value="cpu">CPU</option>
                    <option value="cuda">CUDA</option>
                  </select>
                </Field>
                <Field label={t("settings.asr.compute")} htmlFor="asr-compute">
                  <select
                    id="asr-compute"
                    className="select"
                    value={draft.voice.asr.compute_type}
                    onChange={(event) => patch({ voice: { asr: { compute_type: event.target.value } } })}
                  >
                    <option value="int8">int8</option>
                    <option value="float16">float16</option>
                  </select>
                </Field>
              </div>
              <div className="row row--between">
                <span className="field__label">{t("settings.asr.language")}</span>
                <span className="mono">{state.asr.language ?? "zh"}</span>
              </div>
              <div className="row row--between row--wrap">
                <Button variant="secondary" size="sm" icon="download" disabled title={t("settings.asr.downloadUnavailable")}>
                  {t("settings.asr.download")}
                </Button>
                <span className="field__hint">{t("settings.asr.downloadUnavailable")}</span>
              </div>
            </div>
          </Card>

          <Card title={t("settings.about.title")}>
            <div className="stack">
              <div className="row row--between"><span className="field__label">{t("settings.about.version")}</span><span className="mono">{state.app.version}</span></div>
              <div className="row row--between"><span className="field__label">{t("settings.about.platform")}</span><span className="mono">{state.app.os}</span></div>
              <div className="row row--between"><span className="field__label">{t("link.detail.kind")}</span><span>{state.link.kind ? t(`link.control.${state.link.kind}`) : "—"}</span></div>
              <div className="row row--between"><span className="field__label">{t("settings.about.docs")}</span><span className="mono">docs/interfaces.md</span></div>
              <Banner tone="info" icon="shield">{t("settings.about.mockNotice")}</Banner>
            </div>
          </Card>
        </div>
      </div>

      {showReset ? (
        <div className="dialog-backdrop" role="presentation" onClick={() => setShowReset(false)}>
          <div className="dialog" role="dialog" aria-modal="true" aria-labelledby="reset-title" onClick={(event) => event.stopPropagation()}>
            <header className="dialog__header" id="reset-title">{t("settings.reset")}</header>
            <div className="dialog__body">{t("settings.resetConfirm")}</div>
            <footer className="dialog__footer">
              <Button variant="ghost" onClick={() => setShowReset(false)}>{t("common.cancel")}</Button>
              <Button variant="danger" icon="rotate" onClick={() => void reset()}>{t("common.confirm")}</Button>
            </footer>
          </div>
        </div>
      ) : null}
    </div>
  );
}
