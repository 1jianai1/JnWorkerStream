import { useI18n } from "../../../lib/i18n";
import { macroByteEstimate, macroDuration } from "../../../lib/keypad";
import type { MacroStep } from "../../../lib/types";
import { Button, IconButton } from "../../../components/primitives";
import { Icon } from "../../../components/Icon";
import { formatDuration } from "../../../lib/format";

let stepCounter = 100;

export function MacroEditor({
  macro,
  onChange,
  onCaptureStep,
  onAddStep,
  onRecordSequence,
}: {
  macro: MacroStep[];
  onChange: (macro: MacroStep[]) => void;
  onCaptureStep: (stepId: string) => void;
  onAddStep: () => void;
  onRecordSequence: () => void;
}) {
  const { t } = useI18n();
  const bytes = macroByteEstimate(macro);
  const duration = macroDuration(macro);
  const overLimit = bytes > 200;

  const update = (id: string, patch: Partial<MacroStep>) => {
    onChange(macro.map((step) => (step.id === id ? { ...step, ...patch } : step)));
  };

  const remove = (id: string) => onChange(macro.filter((step) => step.id !== id));

  const addDelay = () => {
    stepCounter += 1;
    onChange([...macro, { id: `s${stepCounter}`, kind: "delay", keys: "", holdMs: 0, delayMs: 30 }]);
  };

  return (
    <div className="macro-editor">
      {macro.length === 0 ? (
        <p className="field__hint">{t("keys.panel.macro.empty")}</p>
      ) : (
        <ol className="macro-steps">
          {macro.map((step, index) => (
            <li className="macro-step" key={step.id}>
              <span className="macro-step__grip" aria-hidden="true"><Icon name="grip" size={14} /></span>
              <span className="macro-step__index tabular">{index + 1}</span>
              {step.kind === "keys" ? (
                <>
                  <button
                    type="button"
                    className="macro-keybtn"
                    onClick={() => onCaptureStep(step.id)}
                    title={t("keys.panel.macro.captureHint")}
                  >
                    <Icon name="keyboard" size={14} />
                    <span>{step.keys || t("keys.panel.macro.pressKey")}</span>
                  </button>
                  <label className="macro-num">
                    <span>{t("keys.panel.macro.hold")} (ms)</span>
                    <input
                      className="input tabular"
                      type="number"
                      min={1}
                      max={5000}
                      value={step.holdMs}
                      onChange={(event) => update(step.id, { holdMs: Number(event.target.value) })}
                    />
                  </label>
                </>
              ) : (
                <span className="macro-step__delay">
                  <Icon name="clock" size={14} />
                  {t("keys.panel.macro.delayStep", { ms: step.delayMs })}
                </span>
              )}
              <label className="macro-num">
                <span>{t("keys.panel.macro.delay")} (ms)</span>
                <input
                  className="input tabular"
                  type="number"
                  min={0}
                  max={5000}
                  value={step.delayMs}
                  onChange={(event) => update(step.id, { delayMs: Number(event.target.value) })}
                />
              </label>
              <IconButton icon="trash" label={t("keys.panel.macro.removeStep")} onClick={() => remove(step.id)} />
            </li>
          ))}
        </ol>
      )}

      <div className="row row--wrap">
        <Button variant="secondary" size="sm" icon="plus" onClick={onAddStep} data-testid="macro-add-step">
          {t("keys.panel.macro.addStep")}
        </Button>
        <Button variant="secondary" size="sm" icon="keyboard" onClick={onRecordSequence} data-testid="macro-record">
          {t("keys.panel.macro.recordSequence")}
        </Button>
        <Button variant="ghost" size="sm" icon="clock" onClick={addDelay}>
          {t("keys.panel.macro.addDelay")}
        </Button>
      </div>

      {macro.length > 0 ? (
        <div className="macro-preview">
          <span className="eyebrow">{t("keys.panel.macro.preview")}</span>
          <div className="macro-timeline" aria-hidden="true">
            {macro.map((step) => (
              <span
                key={step.id}
                className={`macro-bar macro-bar--${step.kind}`}
                style={{ flexGrow: Math.max(1, step.kind === "delay" ? step.delayMs : step.holdMs + step.delayMs) }}
                title={step.kind === "delay"
                  ? t("keys.panel.macro.delayStep", { ms: step.delayMs })
                  : t("keys.panel.macro.keysStep", { keys: step.keys, hold: step.holdMs })}
              />
            ))}
          </div>
        </div>
      ) : null}

      <div className="row row--between macro-summary">
        <span className="muted">
          {t("keys.panel.macro.total")} <strong className="tabular">{formatDuration(duration)}</strong>
        </span>
        <span className={`tabular ${overLimit ? "text-danger" : "muted"}`}>
          {t("keys.panel.macro.bytes", { bytes })}
        </span>
      </div>
      {overLimit ? <p className="field__error" role="alert">{t("keys.panel.macro.overLimit")}</p> : null}
    </div>
  );
}
