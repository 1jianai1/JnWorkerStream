import { useLayoutEffect, useRef, useState } from "react";
import { useI18n } from "../../../lib/i18n";
import type { KeyConfig, KeyRoute, KeyTrigger } from "../../../lib/types";
import { isConfigured, validateKeyConfig } from "../../../lib/keypad";
import { triggerAvailable } from "../../../lib/status";
import { Banner, Button, Field, Segmented } from "../../../components/primitives";
import { Icon, type IconName } from "../../../components/Icon";
import { MacroEditor } from "./MacroEditor";
import { ActionForm } from "./ActionForm";

const routeMeta: Array<{ value: KeyRoute; icon: IconName; titleKey: string; descKey: string }> = [
  { value: "devicePassthrough", icon: "arrowRight", titleKey: "keys.panel.route.devicePassthrough", descKey: "keys.panel.route.devicePassthroughDesc" },
  { value: "deviceMacro", icon: "keyboard", titleKey: "keys.panel.route.deviceMacro", descKey: "keys.panel.route.deviceMacroDesc" },
  { value: "host", icon: "monitor", titleKey: "keys.panel.route.host", descKey: "keys.panel.route.hostDesc" },
];

export function KeyPanel({
  keyId,
  config,
  saved,
  online,
  pendingPush,
  saving,
  onChange,
  onRoute,
  onCapture,
  onCaptureStep,
  onAddMacroStep,
  onRecordSequence,
  onApply,
  onTest,
  onDiscard,
}: {
  keyId: number;
  config: KeyConfig;
  saved: KeyConfig;
  online: boolean;
  pendingPush: number;
  saving: boolean;
  onChange: (patch: Partial<KeyConfig>) => void;
  onRoute: (route: KeyRoute) => void;
  onCapture: () => void;
  onCaptureStep: (stepId: string) => void;
  onAddMacroStep: () => void;
  onRecordSequence: () => void;
  onApply: () => void;
  onTest: () => void;
  onDiscard: () => void;
}) {
  const { t } = useI18n();
  const validation = validateKeyConfig(config);
  const target = validation.target;
  const dirty = JSON.stringify(config) !== JSON.stringify(saved);
  const dirtyFields = [config.route, config.usage, JSON.stringify(config.macro), JSON.stringify(config.action), config.trigger]
    .filter((value, index) => {
      const savedValues = [saved.route, saved.usage, JSON.stringify(saved.macro), JSON.stringify(saved.action), saved.trigger];
      return value !== savedValues[index];
    }).length;

  const targetLabel = t(`keys.panel.headerTarget.${target}`);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const [canScroll, setCanScroll] = useState(false);
  const [atBottom, setAtBottom] = useState(true);

  useLayoutEffect(() => {
    const element = scrollRef.current;
    if (!element) return;
    setCanScroll(element.scrollHeight > element.clientHeight + 4);
    setAtBottom(element.scrollHeight - (element.scrollTop + element.clientHeight) < 8);
  }, [config, online, pendingPush, targetLabel]);
  const triggerOptions: Array<{ value: KeyTrigger; label: string; disabled?: boolean }> = [
    { value: "tap", label: t("keys.panel.trigger.tap") },
    { value: "double", label: t("keys.panel.trigger.double") },
    { value: "long", label: t("keys.panel.trigger.long") },
    { value: "repeat", label: t("keys.panel.trigger.repeat") },
  ];

  return (
    <div className="key-panel" data-testid="key-panel">
      <header className="key-panel__header">
        <div className="stack" style={{ gap: 2 }}>
          <div className="row" style={{ gap: 8 }}>
            <h2 className="key-panel__title">K{keyId}</h2>
            <span className="tag">{t("keys.stage.silkscreenPending")}</span>
          </div>
          <span className="field__hint">
            {isConfigured(config) ? t(`keys.panel.route.${config.route}`) : t("keys.legend.unconfigured")}
          </span>
        </div>
        <span className="tag tag--info">{targetLabel}</span>
      </header>

      <div className="key-panel__scroll-wrap">
        <div
          className="key-panel__scroll"
          ref={scrollRef}
          onScroll={(event) => {
            const element = event.currentTarget;
            setAtBottom(element.scrollHeight - (element.scrollTop + element.clientHeight) < 8);
          }}
        >
        <section className="panel-section">
          <h3 className="panel-section__title">{t("keys.panel.route.title")}</h3>
          <div className="route-options" role="radiogroup" aria-label={t("keys.panel.route.title")}>
            {routeMeta.map((meta) => (
              <button
                key={meta.value}
                type="button"
                role="radio"
                aria-checked={config.route === meta.value}
                className={`route-option${config.route === meta.value ? " is-selected" : ""}`}
                onClick={() => onRoute(meta.value)}
              >
                <span className="route-option__icon"><Icon name={meta.icon} size={16} /></span>
                <span className="route-option__body">
                  <span className="route-option__title">{t(meta.titleKey)}</span>
                  <span className="route-option__desc">{t(meta.descKey)}</span>
                </span>
              </button>
            ))}
          </div>
          <p className="field__hint">{t("keys.panel.route.managedHint")}</p>
        </section>

        {validation.code === "conflict" ? (
          <Banner tone="danger" title={t("keys.panel.hints.conflictHostMacro")} />
        ) : null}
        {validation.code === "overLimit" ? (
          <Banner tone="danger" title={t("keys.panel.hints.overLimit")} />
        ) : null}
        {validation.code === "targetRequired" ? (
          <Banner tone="warning" title={t("keys.panel.action.targetRequired")} />
        ) : null}

        <section className="panel-section">
          <h3 className="panel-section__title">{t("keys.panel.content.title")}</h3>
          {config.route === "devicePassthrough" ? (
            <div className="stack">
              <Field label={t("keys.panel.passthrough.outputKey")}>
                <button type="button" className="key-binding" onClick={onCapture}>
                  <span className="key-binding__icon"><Icon name="keyboard" size={16} /></span>
                  <span className="key-binding__value">
                    {config.captureLabel || config.usage || t("keys.panel.passthrough.pressKey")}
                  </span>
                  <span className="key-binding__action">{t("keys.panel.passthrough.capture")}</span>
                </button>
              </Field>
              <div className="key-binding__meta mono">
                <span>usage {config.usage || "—"}</span>
                <span>modifier {config.modifier === "none" ? "0x00" : config.modifier || "0x00"}</span>
              </div>
              <p className="field__hint">{t("keys.panel.passthrough.hint")}</p>
            </div>
          ) : null}

          {config.route === "deviceMacro" ? (
            <MacroEditor
              macro={config.macro}
              onChange={(macro) => onChange({ macro })}
              onCaptureStep={onCaptureStep}
              onAddStep={onAddMacroStep}
              onRecordSequence={onRecordSequence}
            />
          ) : null}

          {config.route === "host" ? (
            <ActionForm
              action={config.action}
              onChange={(action) => onChange({ action })}
              showErrors={config.action.target.trim() === ""}
            />
          ) : null}
        </section>

        <section className="panel-section">
          <h3 className="panel-section__title">{t("keys.panel.trigger.title")}</h3>
          <Segmented
            ariaLabel={t("keys.panel.trigger.title")}
            value={config.trigger}
            options={triggerOptions.map((option) => ({
              ...option,
              disabled: !triggerAvailable(config.route),
            }))}
            onChange={(trigger) => onChange({ trigger })}
          />
          <p className="field__hint">
            {triggerAvailable(config.route) ? t("keys.panel.trigger.hint") : t("keys.panel.trigger.deviceOnly")}
          </p>
        </section>

        {dirty ? (
          <p className="field__hint">{t("keys.panel.hints.unsaved", { key: `K${keyId}`, count: Math.max(1, dirtyFields) })}</p>
        ) : null}
        </div>
      </div>
      {canScroll && !atBottom ? (
        <button
          type="button"
          className="key-panel__more"
          onClick={() => scrollRef.current?.scrollBy({ top: 180, behavior: "smooth" })}
        >
          {t("common.scrollMore")}
          <Icon name="chevronDown" size={13} />
        </button>
      ) : null}

      <footer className="key-panel__footer">
        <div className="row row--between row--wrap">
          <span className="field__hint">
            {online ? t("keys.panel.footer.target", { target: targetLabel }) : t("keys.panel.footer.offline")}
          </span>
          <span className={`tag ${dirty || pendingPush > 0 ? "tag--warning" : ""}`}>
            {dirty
              ? t("keys.panel.footer.dirty", { count: Math.max(1, dirtyFields) })
              : pendingPush > 0
                ? t("keys.panel.footer.pending", { count: pendingPush })
                : t("keys.panel.footer.clean")}
          </span>
        </div>
        <div className="row key-panel__actions">
          <Button variant="ghost" size="sm" onClick={onDiscard} disabled={!dirty}>
            {t("common.discard")}
          </Button>
          <span className="grow" />
          <Button variant="secondary" icon="play" onClick={onTest} data-testid="key-test">
            {t("keys.panel.footer.test")}
          </Button>
          <Button
            variant={online ? "primary" : "secondary"}
            icon="check"
            loading={saving}
            disabled={!validation.ok || saving}
            onClick={onApply}
            data-testid="key-apply"
          >
            {online ? t("keys.panel.footer.apply") : t("keys.panel.footer.offlineApply")}
          </Button>
        </div>
      </footer>
    </div>
  );
}
