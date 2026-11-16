import { useRef } from "react";
import { useI18n } from "../../../lib/i18n";
import type { KeypadStageProps } from "./types";
import { keyLabel, neighborKey } from "./types";

const MARGIN = 40;
const CELL_X = 150;
const CELL_Y = 130;
const KEY_W = 124;
const KEY_H = 104;

export function KeypadStage2D({ layout, visuals, selected, onSelect, describeKey }: KeypadStageProps) {
  const { locale } = useI18n();
  const containerRef = useRef<HTMLDivElement | null>(null);

  const focusKey = (keyId: number) => {
    const element = containerRef.current?.querySelector<SVGGElement>(`[data-key-id="${keyId}"]`);
    element?.focus();
  };

  const handleKeyDown = (event: React.KeyboardEvent<SVGGElement>, keyId: number) => {
    const map = {
      ArrowLeft: "left",
      ArrowRight: "right",
      ArrowUp: "up",
      ArrowDown: "down",
    } as const;
    if (event.key === "Escape") {
      event.preventDefault();
      onSelect(null);
      return;
    }
    if (event.key === "Home" || event.key === "End") {
      event.preventDefault();
      const entry = layout.keys.find((key) => key.key_id === keyId);
      const row = entry?.row ?? 0;
      const target = event.key === "Home"
        ? layout.keys.find((key) => key.row === row && key.col === 0)
        : layout.keys.find((key) => key.row === row && key.col === layout.grid.cols - 1);
      if (target) {
        onSelect(target.key_id);
        focusKey(target.key_id);
      }
      return;
    }
    const direction = map[event.key as keyof typeof map];
    if (!direction) return;
    event.preventDefault();
    const next = neighborKey(layout, keyId, direction);
    if (next !== null) {
      onSelect(next);
      focusKey(next);
    }
  };

  return (
    <div className="keypad-stage keypad-stage--2d" ref={containerRef}>
      <svg
        className="keypad-svg"
        viewBox={`0 0 ${MARGIN * 2 + (layout.grid.cols - 1) * CELL_X + KEY_W} ${MARGIN * 2 + (layout.grid.rows - 1) * CELL_Y + KEY_H}`}
        role="group"
        aria-label="keypad"
      >
        <rect
          x={MARGIN - 18}
          y={MARGIN - 18}
          width={(layout.grid.cols - 1) * CELL_X + KEY_W + 36}
          height={(layout.grid.rows - 1) * CELL_Y + KEY_H + 36}
          rx={14}
          className="keypad-base"
        />
        {layout.keys.map((entry) => {
          const visual = visuals[entry.key_id];
          const x = MARGIN + entry.col * CELL_X;
          const y = MARGIN + entry.row * CELL_Y;
          const isSelected = selected === entry.key_id;
          return (
            <g
              key={entry.key_id}
              className={`keycap-group${isSelected ? " is-selected" : ""}`}
              tabIndex={0}
              role="button"
              aria-pressed={isSelected}
              aria-label={describeKey(entry.key_id)}
              data-key-id={entry.key_id}
              data-testid={`key-K${entry.key_id}`}
              onClick={() => onSelect(entry.key_id)}
              onKeyDown={(event) => handleKeyDown(event, entry.key_id)}
            >
              <rect x={x} y={y} width={KEY_W} height={KEY_H} rx={8} className="keycap" />
              <rect
                x={x}
                y={y}
                width={KEY_W}
                height={KEY_H}
                rx={8}
                className="keycap-edge"
                aria-hidden="true"
              />
              {isSelected ? (
                <rect x={x + 2} y={y + 2} width={KEY_W - 4} height={KEY_H - 4} rx={6} className="keycap-selected" aria-hidden="true" />
              ) : null}
              <text x={x + KEY_W / 2} y={y + KEY_H / 2 + 6} textAnchor="middle" className="keycap-label">
                {keyLabel(layout, entry.key_id, locale)}
              </text>
              {visual?.configured && visual.route !== "devicePassthrough" ? (
                <circle
                  cx={x + KEY_W - 13}
                  cy={y + KEY_H - 13}
                  r={4}
                  className={`keycap-dot keycap-dot--${visual.route === "deviceMacro" ? "macro" : "host"}`}
                  aria-hidden="true"
                />
              ) : null}
              {visual?.conflict ? (
                <path
                  d={`M ${x + KEY_W - 20} ${y + 12} l 9 16 h -18 z`}
                  className="keycap-conflict"
                  aria-hidden="true"
                />
              ) : null}
              {visual?.dirty ? (
                <circle cx={x + 13} cy={y + 13} r={3.5} className="keycap-dirty" aria-hidden="true" />
              ) : null}
            </g>
          );
        })}
      </svg>
    </div>
  );
}
