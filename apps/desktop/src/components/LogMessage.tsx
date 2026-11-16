import { useI18n } from "../lib/i18n";
import type { LogEntry } from "../lib/types";

export function LogMessage({ entry }: { entry: LogEntry }) {
  const { t } = useI18n();
  if (entry.textKey) return <>{t(entry.textKey, entry.params)}</>;
  return <>{entry.text ?? ""}</>;
}
