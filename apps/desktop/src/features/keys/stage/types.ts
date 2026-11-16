import type { StageKeyVisual } from "../../../lib/keypad";

export interface KeyLayoutEntry {
  key_id: number;
  row: number;
  col: number;
  label_zh: string;
  label_en: string;
  size: number;
}

export interface KeyLayout {
  model: string;
  grid: { rows: number; cols: number };
  keys: KeyLayoutEntry[];
}

export interface KeypadStageProps {
  layout: KeyLayout;
  visuals: StageKeyVisual[];
  selected: number | null;
  onSelect: (keyId: number | null) => void;
  describeKey: (keyId: number) => string;
  onUnavailable?: () => void;
}

export function keyLabel(layout: KeyLayout, keyId: number, locale: "zh-CN" | "en-US"): string {
  const entry = layout.keys.find((key) => key.key_id === keyId);
  if (!entry) return `K${keyId}`;
  return locale === "zh-CN" ? entry.label_zh : entry.label_en;
}

export function neighborKey(layout: KeyLayout, keyId: number, direction: "left" | "right" | "up" | "down"): number | null {
  const entry = layout.keys.find((key) => key.key_id === keyId);
  if (!entry) return null;
  const target = {
    row: entry.row + (direction === "down" ? 1 : direction === "up" ? -1 : 0),
    col: entry.col + (direction === "right" ? 1 : direction === "left" ? -1 : 0),
  };
  if (target.row < 0 || target.row >= layout.grid.rows || target.col < 0 || target.col >= layout.grid.cols) {
    return null;
  }
  return layout.keys.find((key) => key.row === target.row && key.col === target.col)?.key_id ?? null;
}
