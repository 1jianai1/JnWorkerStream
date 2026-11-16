import {
  createContext,
  useCallback,
  useContext,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { Icon } from "../components/Icon";
import { useI18n } from "./i18n";

export type ToastLevel = "info" | "success" | "warning" | "danger";

interface ToastItem {
  id: number;
  level: ToastLevel;
  titleKey: string;
  textKey?: string;
  params?: Record<string, string | number>;
}

interface ToastApi {
  push: (toast: Omit<ToastItem, "id">) => void;
}

const ToastContext = createContext<ToastApi | null>(null);

export function ToastProvider({ children }: { children: ReactNode }) {
  const [items, setItems] = useState<ToastItem[]>([]);
  const nextId = useRef(1);
  const timers = useRef(new Map<number, number>());

  const remove = useCallback((id: number) => {
    setItems((current) => current.filter((item) => item.id !== id));
    const timer = timers.current.get(id);
    if (timer !== undefined) window.clearTimeout(timer);
    timers.current.delete(id);
  }, []);

  const push = useCallback<ToastApi["push"]>((toast) => {
    const id = nextId.current;
    nextId.current += 1;
    setItems((current) => [...current.slice(-2), { ...toast, id }]);
    const timer = window.setTimeout(() => {
      setItems((current) => current.filter((item) => item.id !== id));
      timers.current.delete(id);
    }, 4000);
    timers.current.set(id, timer);
  }, []);

  const api = useMemo<ToastApi>(() => ({ push }), [push]);

  return (
    <ToastContext.Provider value={api}>
      {children}
      <div className="toast-region" role="region" aria-live="polite">
        {items.map((item) => (
          <ToastCard key={item.id} item={item} onDismiss={() => remove(item.id)} />
        ))}
      </div>
    </ToastContext.Provider>
  );
}

function ToastCard({ item, onDismiss }: { item: ToastItem; onDismiss: () => void }) {
  const { t } = useI18n();
  return (
    <div className={`toast toast--${item.level}`} role="status">
      <Icon name={item.level === "danger" ? "alertTriangle" : item.level === "success" ? "check" : "info"} size={16} />
      <div className="toast__body">
        <span className="toast__title">{t(item.titleKey)}</span>
        {item.textKey ? <span className="toast__text">{t(item.textKey, item.params)}</span> : null}
      </div>
      <button type="button" className="icon-btn" onClick={onDismiss} aria-label={t("common.dismiss")}>
        <Icon name="x" size={14} />
      </button>
    </div>
  );
}

export function useToast(): ToastApi {
  const value = useContext(ToastContext);
  if (!value) throw new Error("useToast must be used inside ToastProvider");
  return value;
}
