import { useState } from "react";
import { useI18n } from "../../../lib/i18n";
import type { ActionKind, HostAction } from "../../../lib/types";
import { Button, Field } from "../../../components/primitives";
import { Icon } from "../../../components/Icon";

const kinds: ActionKind[] = ["launch", "openPath", "openUrl", "script"];

export function ActionForm({
  action,
  onChange,
  showErrors,
}: {
  action: HostAction;
  onChange: (action: HostAction) => void;
  showErrors: boolean;
}) {
  const { t } = useI18n();
  const [dragging, setDragging] = useState(false);
  const targetMissing = action.target.trim() === "";
  const needsTarget = action.kind !== "script" || action.target.trim() === "";

  return (
    <div className="action-form">
      <Field label={t("keys.panel.action.kind")} htmlFor="action-kind">
        <select
          id="action-kind"
          className="select"
          value={action.kind}
          onChange={(event) => onChange({ ...action, kind: event.target.value as ActionKind })}
        >
          {kinds.map((kind) => (
            <option key={kind} value={kind}>{t(`keys.panel.action.${kind}`)}</option>
          ))}
        </select>
      </Field>

      <Field
        label={t("keys.panel.action.target")}
        htmlFor="action-target"
        required
        error={showErrors && targetMissing && needsTarget ? t("keys.panel.action.targetRequired") : undefined}
      >
        <div className="row">
          <input
            id="action-target"
            className="input mono grow"
            value={action.target}
            placeholder={action.kind === "openUrl" ? "https://example.com" : t("keys.panel.action.targetPlaceholder")}
            onChange={(event) => onChange({ ...action, target: event.target.value })}
          />
          <Button variant="secondary" size="sm" icon="folder" disabled title={t("keys.panel.action.dropzoneHint")}>
            {t("keys.panel.action.browse")}
          </Button>
        </div>
      </Field>

      <Field label={t("keys.panel.action.args")} htmlFor="action-args">
        <input
          id="action-args"
          className="input mono"
          value={action.args}
          onChange={(event) => onChange({ ...action, args: event.target.value })}
        />
      </Field>

      {action.kind === "script" ? (
        <div className="grid-2">
          <Field label={t("keys.panel.action.shell")} htmlFor="action-shell">
            <select
              id="action-shell"
              className="select"
              value={action.shell}
              onChange={(event) => onChange({ ...action, shell: event.target.value as HostAction["shell"] })}
            >
              <option value="powershell">PowerShell</option>
              <option value="pwsh">pwsh</option>
              <option value="cmd">CMD</option>
            </select>
          </Field>
          <Field label={t("keys.panel.action.timeout")} htmlFor="action-timeout">
            <div className="input-group">
              <input
                id="action-timeout"
                className="input tabular"
                type="number"
                min={100}
                max={60000}
                step={100}
                value={action.timeoutMs}
                onChange={(event) => onChange({ ...action, timeoutMs: Number(event.target.value) })}
              />
              <span className="input-group__suffix">ms</span>
            </div>
          </Field>
        </div>
      ) : null}

      <div
        className={`dropzone${dragging ? " is-dragging" : ""}`}
        onDragOver={(event) => {
          event.preventDefault();
          setDragging(true);
        }}
        onDragLeave={() => setDragging(false)}
        onDrop={(event) => {
          event.preventDefault();
          setDragging(false);
          const file = event.dataTransfer.files[0];
          if (!file) return;
          onChange({ ...action, kind: "launch", target: file.name });
        }}
      >
        <Icon name="file" size={20} />
        <span>{t("keys.panel.action.dropzone")}</span>
        <span className="field__hint">{t("keys.panel.action.dropzoneHint")}</span>
      </div>
    </div>
  );
}
