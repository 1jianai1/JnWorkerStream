import {
  useCallback,
  useRef,
  useState,
  type CSSProperties,
  type HTMLAttributes,
  type KeyboardEvent as ReactKeyboardEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";

export type SplitEdge = "left" | "right";

interface ResizableOptions {
  storageKey: string;
  initial: number | (() => number);
  min: number;
  max: number;
  edge: SplitEdge;
  label: string;
  cssVar?: string;
}

interface DragState {
  pointerId: number;
  startX: number;
  startWidth: number;
}

function initialWidth(initial: number | (() => number)): number {
  return typeof initial === "function" ? initial() : initial;
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, Math.round(value)));
}

/**
 * Draggable inner-panel width with pointer + keyboard support and localStorage
 * persistence. Apply `style` to the grid container and render `SplitHandle`
 * inside the resizable panel with the same `edge`.
 */
export function useResizablePanel({ storageKey, initial, min, max, edge, label, cssVar = "--split-width" }: ResizableOptions) {
  const fallback = initialWidth(initial);
  const [width, setWidth] = useState<number>(() => {
    try {
      const stored = Number(localStorage.getItem(`jnws.ui.split.${storageKey}`));
      return Number.isFinite(stored) && stored > 0 ? clamp(stored, min, max) : clamp(fallback, min, max);
    } catch {
      return clamp(fallback, min, max);
    }
  });
  const [resizing, setResizing] = useState(false);
  const drag = useRef<DragState | null>(null);
  const widthRef = useRef(width);
  widthRef.current = width;

  const persist = useCallback((value: number) => {
    try {
      localStorage.setItem(`jnws.ui.split.${storageKey}`, String(value));
    } catch {
      /* best effort */
    }
  }, [storageKey]);

  const apply = useCallback((value: number) => {
    const next = clamp(value, min, max);
    setWidth(next);
    return next;
  }, [min, max]);

  const onPointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    event.preventDefault();
    const target = event.currentTarget;
    target.setPointerCapture(event.pointerId);
    drag.current = { pointerId: event.pointerId, startX: event.clientX, startWidth: widthRef.current };
    setResizing(true);
    document.body.classList.add("is-resizing");
  };

  const onPointerMove = (event: ReactPointerEvent<HTMLDivElement>) => {
    const state = drag.current;
    if (!state || state.pointerId !== event.pointerId) return;
    const delta = event.clientX - state.startX;
    apply(edge === "right" ? state.startWidth + delta : state.startWidth - delta);
  };

  const finish = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (!drag.current) return;
    drag.current = null;
    setResizing(false);
    document.body.classList.remove("is-resizing");
    persist(widthRef.current);
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  };

  const onKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    event.preventDefault();
    const step = event.shiftKey ? 40 : 12;
    const direction = event.key === "ArrowRight" ? 1 : -1;
    const signed = edge === "right" ? direction : -direction;
    persist(apply(widthRef.current + signed * step));
  };

  const reset = () => {
    persist(apply(fallback));
  };

  return {
    width,
    resizing,
    style: { [cssVar]: `${width}px` } as CSSProperties,
    handleProps: {
      role: "separator" as const,
      "aria-orientation": "vertical" as const,
      "aria-label": label,
      "aria-valuemin": min,
      "aria-valuemax": max,
      "aria-valuenow": width,
      tabIndex: 0,
      title: label,
      onPointerDown,
      onPointerMove,
      onPointerUp: finish,
      onPointerCancel: finish,
      onKeyDown,
      onDoubleClick: reset,
      "data-resizing": resizing ? "true" : "false",
    },
  };
}

export function SplitHandle({
  edge,
  handleProps,
}: {
  edge: SplitEdge;
  handleProps: HTMLAttributes<HTMLDivElement>;
}) {
  return (
    <div className={`split-handle split-handle--${edge}`} {...handleProps}>
      <span aria-hidden="true" />
    </div>
  );
}
