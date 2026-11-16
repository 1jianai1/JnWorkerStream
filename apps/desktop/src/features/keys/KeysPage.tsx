import { useEffect, useMemo, useRef, useState } from "react";
import layoutJson from "../../assets/layout.json";
import { useAppState } from "../../lib/store";
import { backend, runtime } from "../../lib/ipc";
import { useI18n } from "../../lib/i18n";
import { useToast } from "../../lib/toast";
import { Banner, Card, EmptyState, IconButton, PageHeader, Segmented } from "../../components/primitives";
import { KeyPanel } from "./panel/KeyPanel";
import { KeyCaptureDialog } from "./KeyCaptureDialog";
import { formatModifierMask, formatUsage, type CapturedCombo } from "../../lib/key-capture";
import { SplitHandle, useResizablePanel } from "../../components/ResizablePanel";
import { KeypadStage2D } from "./stage/KeypadStage2D";
import { KeypadStage3D } from "./stage/KeypadStage3D";
import { keyLabel, type KeyLayout } from "./stage/types";
import { isConfigured, stageVisual, validateKeyConfig, type StageKeyVisual } from "../../lib/keypad";
import type { KeyConfig, KeyRoute } from "../../lib/types";

const layout = layoutJson as KeyLayout;

function clone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}

function webgl2Available(): boolean {
  try {
    const canvas = document.createElement("canvas");
    return Boolean(canvas.getContext("webgl2"));
  } catch {
    return false;
  }
}

function isEditable(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName) || target.isContentEditable;
}

type CaptureTarget =
  | { type: "passthrough" }
  | { type: "macroStep"; stepId: string }
  | { type: "sequence" };

export function KeysPage() {
  const state = useAppState();
  const { t, locale } = useI18n();
  const { push } = useToast();
  const saved = state.keys.configs;
  const [drafts, setDrafts] = useState<KeyConfig[]>(() => saved.map((config) => clone(config)));
  const [selected, setSelected] = useState<number | null>(null);
  const [saving, setSaving] = useState(false);
  const [captureTarget, setCaptureTarget] = useState<CaptureTarget | null>(null);
  const [force2d, setForce2d] = useState(false);
  const [switchedTo2d, setSwitchedTo2d] = useState(false);
  const [stageNonce, setStageNonce] = useState(0);
  const undoRef = useRef<Array<{ keyId: number; config: KeyConfig }>>([]);
  const panel = useResizablePanel({
    storageKey: "keyPanel",
    initial: () => (window.innerWidth >= 1600 ? 440 : 400),
    min: 340,
    max: 460,
    edge: "left",
    label: t("layout.resizeKeyPanel"),
  });

  const selectedDraft = selected !== null ? drafts[selected] : undefined;
  const selectedSaved = selected !== null ? saved[selected] : undefined;

  const dirtyCount = useMemo(
    () => drafts.reduce((count, draft, index) => (JSON.stringify(draft) !== JSON.stringify(saved[index]) ? count + 1 : count), 0),
    [drafts, saved],
  );

  const visuals: StageKeyVisual[] = useMemo(
    () => drafts.map((draft, index) => stageVisual(draft, saved[index] ?? draft, state.keys.device_online)),
    [drafts, saved, state.keys.device_online],
  );

  const urlRenderer = runtime.rendererOverride === "2d" || runtime.rendererOverride === "3d" ? runtime.rendererOverride : null;
  const configuredRenderer = urlRenderer ?? state.settings.ui.keypad.renderer;
  const resolvedRenderer = configuredRenderer === "auto"
    ? (webgl2Available() && !force2d ? "3d" : "2d")
    : force2d ? "2d" : configuredRenderer;

  const pushUndo = (keyId: number) => {
    const config = drafts[keyId];
    if (!config) return;
    undoRef.current = [...undoRef.current.slice(-49), { keyId, config: clone(config) }];
  };

  const patchKey = (patch: Partial<KeyConfig>) => {
    if (selected === null) return;
    pushUndo(selected);
    setDrafts((current) => current.map((config, index) => (index === selected ? { ...config, ...patch } : config)));
  };

  const changeRoute = (route: KeyRoute) => {
    if (selected === null) return;
    pushUndo(selected);
    setDrafts((current) => current.map((config, index) => (index === selected ? { ...config, route } : config)));
  };

  const discardCurrent = () => {
    if (selected === null || !selectedSaved) return;
    setDrafts((current) => current.map((config, index) => (index === selected ? clone(selectedSaved) : config)));
  };

  const applyCurrent = async () => {
    if (selected === null || !selectedDraft) return;
    const validation = validateKeyConfig(selectedDraft);
    if (!validation.ok) return;
    setSaving(true);
    try {
      const result = await backend.applyKey(selected, selectedDraft);
      const target = t(`keys.panel.headerTarget.${result.target}`);
      if (result.applied) {
        push({
          level: "success",
          titleKey: "keys.toast.appliedTitle",
          textKey: "keys.toast.appliedText",
          params: { key: `K${selected}`, target },
        });
      } else {
        push({ level: "warning", titleKey: "keys.toast.savedLocalTitle", textKey: "keys.toast.savedLocalText" });
      }
    } finally {
      setSaving(false);
    }
  };

  const testCurrent = () => {
    if (selected === null) return;
    push({ level: "info", titleKey: "keys.toast.testTitle", textKey: "keys.toast.testText", params: { key: `K${selected}` } });
  };

  const openPassthroughCapture = () => {
    if (selected === null) return;
    setCaptureTarget({ type: "passthrough" });
  };

  const openStepCapture = (stepId: string) => {
    if (selected === null) return;
    setCaptureTarget({ type: "macroStep", stepId });
  };

  const addMacroStep = () => {
    if (selected === null || !selectedDraft) return;
    const id = `s${Date.now()}-${selectedDraft.macro.length + 1}`;
    pushUndo(selected);
    setDrafts((current) => current.map((config, index) => (
      index === selected
        ? { ...config, macro: [...config.macro, { id, kind: "keys", keys: "", holdMs: 20, delayMs: 30 }] }
        : config
    )));
    setCaptureTarget({ type: "macroStep", stepId: id });
  };

  const openSequenceCapture = () => {
    if (selected === null) return;
    setCaptureTarget({ type: "sequence" });
  };

  const commitSingleCombo = (combo: CapturedCombo) => {
    if (!captureTarget || captureTarget.type === "sequence") return;
    if (captureTarget.type === "passthrough") {
      patchKey({
        usage: formatUsage(combo.usage),
        modifier: formatModifierMask(combo.modifier),
        captureLabel: combo.label,
      });
    } else {
      const macro = (selectedDraft?.macro ?? []).map((step) => (
        step.id === captureTarget.stepId
          ? { ...step, keys: combo.label, usage: combo.usage, modifier: combo.modifier }
          : step
      ));
      patchKey({ macro });
    }
    setCaptureTarget(null);
  };

  const commitSequence = (combos: CapturedCombo[]) => {
    if (!selectedDraft) return;
    const start = selectedDraft.macro.length;
    const steps = combos.map((combo, index) => ({
      id: `s${Date.now()}-${start + index}`,
      kind: "keys" as const,
      keys: combo.label,
      usage: combo.usage,
      modifier: combo.modifier,
      holdMs: 20,
      delayMs: 30,
    }));
    patchKey({ macro: [...selectedDraft.macro, ...steps] });
    setCaptureTarget(null);
  };

  const undo = () => {
    const last = undoRef.current.pop();
    if (!last) return;
    setDrafts((current) => current.map((config, index) => (index === last.keyId ? last.config : config)));
    setSelected(last.keyId);
  };

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key === "Enter") {
        event.preventDefault();
        if (selected !== null && selectedDraft && validateKeyConfig(selectedDraft).ok && !saving) void applyCurrent();
        return;
      }
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "z" && !isEditable(event.target)) {
        event.preventDefault();
        undo();
        return;
      }
      if (event.key === "Escape" && !isEditable(event.target)) {
        setSelected(null);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const describeKey = (keyId: number) => {
    const draft = drafts[keyId];
    if (!draft) return `K${keyId}`;
    const routeLabel = t(`keys.panel.route.${draft.route}`);
    const stateLabel = isConfigured(draft) ? routeLabel : t("keys.legend.unconfigured");
    const conflict = validateKeyConfig(draft).ok ? "" : `, ${t("keys.legend.conflict")}`;
    return `${keyLabel(layout, keyId, locale)}, ${stateLabel}${conflict}`;
  };

  const changeRenderer = async (value: "auto" | "3d" | "2d") => {
    setForce2d(false);
    setSwitchedTo2d(false);
    setStageNonce((current) => current + 1);
    await backend.updateSettings({ ui: { keypad: { renderer: value } } });
  };

  return (
    <div className="page page--keys">
      <PageHeader
        title={t("keys.title")}
        subtitle={t("keys.subtitle")}
        badge={dirtyCount > 0 ? (
          <span
            className="tag tag--warning"
            title={t("keys.panel.hints.unsavedAll")}
          >
            {t("keys.panel.footer.dirty", { count: dirtyCount })}
          </span>
        ) : null}
        actions={
          <>
            <span className="topbar-label">{t("keys.stage.renderer")}</span>
            <Segmented
              ariaLabel={t("keys.stage.renderer")}
              value={configuredRenderer}
              options={[
                { value: "auto", label: t("keys.stage.auto") },
                { value: "3d", label: t("keys.stage.3d") },
                { value: "2d", label: t("keys.stage.2d") },
              ]}
              onChange={(value) => void changeRenderer(value)}
            />
            <IconButton
              icon="rotate"
              label={t("keys.stage.resetView")}
              bordered
              onClick={() => setStageNonce((current) => current + 1)}
            />
          </>
        }
      />

      <div className="page-body page-body--keys">
        {!state.keys.device_online ? (
          <Banner tone="warning" title={t("keys.panel.hints.offline")} />
        ) : null}
        {switchedTo2d ? <Banner tone="info" title={t("keys.stage.switched")} /> : null}

        <div className="keys-layout" style={panel.style}>
          <Card
            className="stage-card"
            title={t("keys.stage.title")}
            subtitle={t("keys.stage.hint")}
            actions={
              <span className="tag">{resolvedRenderer === "3d" ? "3D" : "2D"}</span>
            }
            bodyClassName="stage-card__body"
            flush
          >
            <div className="stage-frame" key={`${resolvedRenderer}-${stageNonce}`}>
              {resolvedRenderer === "3d" ? (
                <KeypadStage3D
                  layout={layout}
                  visuals={visuals}
                  selected={selected}
                  onSelect={setSelected}
                  describeKey={describeKey}
                  onUnavailable={() => {
                    setForce2d(true);
                    setSwitchedTo2d(true);
                  }}
                />
              ) : (
                <KeypadStage2D
                  layout={layout}
                  visuals={visuals}
                  selected={selected}
                  onSelect={setSelected}
                  describeKey={describeKey}
                />
              )}
            </div>
            <div className="stage-legend">
              <span className="legend-item"><span className="legend-swatch legend-swatch--macro" />{t("keys.legend.deviceMacro")}</span>
              <span className="legend-item"><span className="legend-swatch legend-swatch--host" />{t("keys.legend.host")}</span>
              <span className="legend-item"><span className="legend-swatch legend-swatch--none" />{t("keys.legend.unconfigured")}</span>
              <span className="legend-item"><span className="legend-swatch legend-swatch--conflict" />{t("keys.legend.conflict")}</span>
              <span className="legend-spacer" />
              <span className="muted mono">{t("keys.stage.silkscreenPending")}</span>
            </div>
          </Card>

          <div className="resizable-panel key-panel-panel">
          {selected === null || !selectedDraft || !selectedSaved ? (
            <Card className="key-panel key-panel--empty">
              <EmptyState icon="keyboard" title={t("keys.panel.emptyTitle")} text={t("keys.panel.emptyText")} />
            </Card>
          ) : (
            <KeyPanel
              keyId={selected}
              config={selectedDraft}
              saved={selectedSaved}
              online={state.keys.device_online}
              pendingPush={state.keys.pending_push}
              saving={saving}
              onChange={patchKey}
              onRoute={changeRoute}
              onCapture={openPassthroughCapture}
              onCaptureStep={openStepCapture}
              onAddMacroStep={addMacroStep}
              onRecordSequence={openSequenceCapture}
              onApply={() => void applyCurrent()}
              onTest={testCurrent}
              onDiscard={discardCurrent}
            />
          )}
            <SplitHandle edge="left" handleProps={panel.handleProps} />
          </div>
        </div>
      </div>

      {captureTarget ? (
        <KeyCaptureDialog
          mode={captureTarget.type === "sequence" ? "sequence" : "single"}
          onCancel={() => setCaptureTarget(null)}
          onSingle={commitSingleCombo}
          onSequence={commitSequence}
        />
      ) : null}

      <div className="sr-only" aria-live="polite">
        {selected === null ? "" : t("keys.panel.footer.target", { target: t(`keys.panel.headerTarget.${validateKeyConfig(selectedDraft ?? drafts[0]!).target}`) })}
      </div>
    </div>
  );
}
