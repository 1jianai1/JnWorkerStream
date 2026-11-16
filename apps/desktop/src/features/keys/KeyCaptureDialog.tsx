import { useEffect, useRef, useState } from "react";
import { useI18n } from "../../lib/i18n";
import { Button } from "../../components/primitives";
import { Icon } from "../../components/Icon";
import { comboFromEvent, isModifierCode, modifierMask, modifierNames, type CapturedCombo } from "../../lib/key-capture";

export type CaptureMode = "single" | "sequence";

export function KeyCaptureDialog({
  mode,
  onCancel,
  onSingle,
  onSequence,
}: {
  mode: CaptureMode;
  onCancel: () => void;
  onSingle?: (combo: CapturedCombo) => void;
  onSequence?: (steps: CapturedCombo[]) => void;
}) {
  const { t } = useI18n();
  const [heldMask, setHeldMask] = useState(0);
  const [steps, setSteps] = useState<CapturedCombo[]>([]);
  const [last, setLast] = useState<CapturedCombo | null>(null);
  const [unsupported, setUnsupported] = useState(false);
  const cardRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    cardRef.current?.focus();
    const onKeyDown = (event: KeyboardEvent) => {
      event.preventDefault();
      event.stopPropagation();
      if (event.key === "Escape") {
        onCancel();
        return;
      }
      setHeldMask(modifierMask(event));
      if (isModifierCode(event.code)) return;
      const combo = comboFromEvent(event);
      if (!combo) {
        setUnsupported(true);
        return;
      }
      setUnsupported(false);
      setLast(combo);
      if (mode === "single") {
        onSingle?.(combo);
        return;
      }
      setSteps((current) => [...current, combo]);
    };
    const onKeyUp = (event: KeyboardEvent) => {
      event.preventDefault();
      setHeldMask(modifierMask(event));
    };
    window.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("keyup", onKeyUp, true);
    return () => {
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("keyup", onKeyUp, true);
    };
  }, [mode, onCancel, onSingle]);

  const held = modifierNames(heldMask);
  const preview = last ? [...held, last.keyLabel].join(" + ") : held.length > 0 ? held.join(" + ") : t("keys.capture.press");

  return (
    <div className="key-capture-backdrop" role="presentation" onMouseDown={(event) => {
      if (event.target === event.currentTarget) onCancel();
    }}>
      <div
        className="key-capture"
        role="dialog"
        aria-modal="true"
        aria-labelledby="key-capture-title"
        tabIndex={-1}
        ref={cardRef}
      >
        <header className="key-capture__header">
          <span className="eyebrow">{t("keys.capture.eyebrow")}</span>
          <h2 id="key-capture-title" className="key-capture__title">
            {mode === "single" ? t("keys.capture.titleSingle") : t("keys.capture.titleSequence")}
          </h2>
          <p className="field__hint">
            {mode === "single" ? t("keys.capture.hintSingle") : t("keys.capture.hintSequence")}
          </p>
        </header>

        <div className="key-capture__preview" aria-live="polite">
          <span className="key-capture__keycap">{preview}</span>
          <span className="key-capture__usage mono">
            {last ? `usage 0x${last.usage.toString(16).toUpperCase().padStart(2, "0")} · modifier 0x${last.modifier.toString(16).toUpperCase().padStart(2, "0")}` : t("keys.capture.waiting")}
          </span>
        </div>

        <div className="key-capture__mods">
          <span className="field__label">{t("keys.capture.modifiers")}</span>
          {held.length === 0 ? (
            <span className="muted">{t("keys.capture.none")}</span>
          ) : (
            held.map((name) => <span className="kbd" key={name}>{name}</span>)
          )}
        </div>

        {unsupported ? <p className="field__error" role="alert">{t("keys.capture.unsupported")}</p> : null}

        {mode === "sequence" ? (
          <div className="key-capture__sequence">
            <div className="row row--between">
              <span className="field__label">{t("keys.capture.recorded")}</span>
              <span className="tag">{t("keys.capture.steps", { count: steps.length })}</span>
            </div>
            {steps.length === 0 ? (
              <p className="field__hint">{t("keys.capture.noSteps")}</p>
            ) : (
              <ol className="key-capture__steps">
                {steps.map((step, index) => (
                  <li key={`${step.label}-${index}`} className="key-capture__step">
                    <span className="macro-step__index tabular">{index + 1}</span>
                    <span className="kbd">{step.label}</span>
                  </li>
                ))}
              </ol>
            )}
            <div className="row row--between">
              <Button
                variant="ghost"
                size="sm"
                icon="rotate"
                disabled={steps.length === 0}
                data-testid="capture-undo"
                onClick={() => setSteps((current) => current.slice(0, -1))}
              >
                {t("keys.capture.undo")}
              </Button>
              <span className="field__hint">{t("keys.capture.esc")}</span>
            </div>
          </div>
        ) : null}

        <footer className="key-capture__footer">
          <Button variant="ghost" onClick={onCancel} data-testid="capture-cancel">{t("common.cancel")}</Button>
          {mode === "sequence" ? (
            <Button
              variant="primary"
              icon="check"
              disabled={steps.length === 0}
              data-testid="capture-done"
              onClick={() => onSequence?.(steps)}
            >
              {t("keys.capture.done")}
            </Button>
          ) : (
            <span className="field__hint"><Icon name="keyboard" size={14} /> {t("keys.capture.esc")}</span>
          )}
        </footer>
      </div>
    </div>
  );
}
