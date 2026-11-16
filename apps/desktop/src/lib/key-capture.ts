/**
 * Browser keydown → HID keyboard usage capture.
 *
 * P1 uses this only to build the UI draft; the authoritative encoder lives in
 * `kb-keymap` (P2f). Modifier bits match `docs/key-routing-and-macros.md`:
 * bit0 Ctrl, bit1 Shift, bit2 Alt, bit3 GUI.
 */

export interface CapturedCombo {
  label: string;
  usage: number;
  modifier: number;
  keyLabel: string;
  modifiers: string[];
}

const MODIFIER_CODES = new Set([
  "ControlLeft",
  "ControlRight",
  "ShiftLeft",
  "ShiftRight",
  "AltLeft",
  "AltRight",
  "MetaLeft",
  "MetaRight",
]);

const CODE_MAP: Record<string, { label: string; usage: number }> = {
  KeyA: { label: "A", usage: 0x04 },
  KeyB: { label: "B", usage: 0x05 },
  KeyC: { label: "C", usage: 0x06 },
  KeyD: { label: "D", usage: 0x07 },
  KeyE: { label: "E", usage: 0x08 },
  KeyF: { label: "F", usage: 0x09 },
  KeyG: { label: "G", usage: 0x0a },
  KeyH: { label: "H", usage: 0x0b },
  KeyI: { label: "I", usage: 0x0c },
  KeyJ: { label: "J", usage: 0x0d },
  KeyK: { label: "K", usage: 0x0e },
  KeyL: { label: "L", usage: 0x0f },
  KeyM: { label: "M", usage: 0x10 },
  KeyN: { label: "N", usage: 0x11 },
  KeyO: { label: "O", usage: 0x12 },
  KeyP: { label: "P", usage: 0x13 },
  KeyQ: { label: "Q", usage: 0x14 },
  KeyR: { label: "R", usage: 0x15 },
  KeyS: { label: "S", usage: 0x16 },
  KeyT: { label: "T", usage: 0x17 },
  KeyU: { label: "U", usage: 0x18 },
  KeyV: { label: "V", usage: 0x19 },
  KeyW: { label: "W", usage: 0x1a },
  KeyX: { label: "X", usage: 0x1b },
  KeyY: { label: "Y", usage: 0x1c },
  KeyZ: { label: "Z", usage: 0x1d },
  Digit1: { label: "1", usage: 0x1e },
  Digit2: { label: "2", usage: 0x1f },
  Digit3: { label: "3", usage: 0x20 },
  Digit4: { label: "4", usage: 0x21 },
  Digit5: { label: "5", usage: 0x22 },
  Digit6: { label: "6", usage: 0x23 },
  Digit7: { label: "7", usage: 0x24 },
  Digit8: { label: "8", usage: 0x25 },
  Digit9: { label: "9", usage: 0x26 },
  Digit0: { label: "0", usage: 0x27 },
  Enter: { label: "Enter", usage: 0x28 },
  NumpadEnter: { label: "Enter", usage: 0x28 },
  Escape: { label: "Esc", usage: 0x29 },
  Backspace: { label: "Backspace", usage: 0x2a },
  Tab: { label: "Tab", usage: 0x2b },
  Space: { label: "Space", usage: 0x2c },
  Minus: { label: "-", usage: 0x2d },
  Equal: { label: "=", usage: 0x2e },
  BracketLeft: { label: "[", usage: 0x2f },
  BracketRight: { label: "]", usage: 0x30 },
  Backslash: { label: "\\", usage: 0x31 },
  Semicolon: { label: ";", usage: 0x33 },
  Quote: { label: "'", usage: 0x34 },
  Backquote: { label: "`", usage: 0x35 },
  Comma: { label: ",", usage: 0x36 },
  Period: { label: ".", usage: 0x37 },
  Slash: { label: "/", usage: 0x38 },
  CapsLock: { label: "Caps", usage: 0x39 },
  F1: { label: "F1", usage: 0x3a },
  F2: { label: "F2", usage: 0x3b },
  F3: { label: "F3", usage: 0x3c },
  F4: { label: "F4", usage: 0x3d },
  F5: { label: "F5", usage: 0x3e },
  F6: { label: "F6", usage: 0x3f },
  F7: { label: "F7", usage: 0x40 },
  F8: { label: "F8", usage: 0x41 },
  F9: { label: "F9", usage: 0x42 },
  F10: { label: "F10", usage: 0x43 },
  F11: { label: "F11", usage: 0x44 },
  F12: { label: "F12", usage: 0x45 },
  Insert: { label: "Insert", usage: 0x49 },
  Home: { label: "Home", usage: 0x4a },
  PageUp: { label: "PageUp", usage: 0x4b },
  Delete: { label: "Delete", usage: 0x4c },
  End: { label: "End", usage: 0x4d },
  PageDown: { label: "PageDown", usage: 0x4e },
  ArrowRight: { label: "→", usage: 0x4f },
  ArrowLeft: { label: "←", usage: 0x50 },
  ArrowDown: { label: "↓", usage: 0x51 },
  ArrowUp: { label: "↑", usage: 0x52 },
  NumLock: { label: "Num", usage: 0x53 },
  NumpadDivide: { label: "Num /", usage: 0x54 },
  NumpadMultiply: { label: "Num *", usage: 0x55 },
  NumpadSubtract: { label: "Num -", usage: 0x56 },
  NumpadAdd: { label: "Num +", usage: 0x57 },
  Numpad1: { label: "Num 1", usage: 0x59 },
  Numpad2: { label: "Num 2", usage: 0x5a },
  Numpad3: { label: "Num 3", usage: 0x5b },
  Numpad4: { label: "Num 4", usage: 0x5c },
  Numpad5: { label: "Num 5", usage: 0x5d },
  Numpad6: { label: "Num 6", usage: 0x5e },
  Numpad7: { label: "Num 7", usage: 0x5f },
  Numpad8: { label: "Num 8", usage: 0x60 },
  Numpad9: { label: "Num 9", usage: 0x61 },
  Numpad0: { label: "Num 0", usage: 0x62 },
  NumpadDecimal: { label: "Num .", usage: 0x63 },
};

export function isModifierCode(code: string): boolean {
  return MODIFIER_CODES.has(code);
}

export function modifierMask(event: Pick<KeyboardEvent, "ctrlKey" | "shiftKey" | "altKey" | "metaKey">): number {
  return (
    (event.ctrlKey ? 0x01 : 0) |
    (event.shiftKey ? 0x02 : 0) |
    (event.altKey ? 0x04 : 0) |
    (event.metaKey ? 0x08 : 0)
  );
}

export function modifierNames(mask: number): string[] {
  const names: string[] = [];
  if (mask & 0x01) names.push("Ctrl");
  if (mask & 0x02) names.push("Shift");
  if (mask & 0x04) names.push("Alt");
  if (mask & 0x08) names.push("Win");
  return names;
}

export function comboFromEvent(event: KeyboardEvent): CapturedCombo | null {
  if (event.repeat || isModifierCode(event.code)) return null;
  const mapped = CODE_MAP[event.code];
  if (!mapped) return null;
  const modifier = modifierMask(event);
  const modifiers = modifierNames(modifier);
  return {
    label: [...modifiers, mapped.label].join("+"),
    usage: mapped.usage,
    modifier,
    keyLabel: mapped.label,
    modifiers,
  };
}

export function formatModifierMask(mask: number): string {
  return `0x${mask.toString(16).toUpperCase().padStart(2, "0")}`;
}

export function formatUsage(usage: number): string {
  return `0x${usage.toString(16).toUpperCase().padStart(2, "0")}`;
}

export function supportedKeyCount(): number {
  return Object.keys(CODE_MAP).length;
}
