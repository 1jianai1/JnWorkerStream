import type { LinkState } from "../lib/types";
import { LINK_TONE } from "../lib/status";
import { StatusPill } from "../components/primitives";
import { useI18n } from "../lib/i18n";

export function LinkStatePill({ state }: { state: LinkState }) {
  const { t } = useI18n();
  return <StatusPill tone={LINK_TONE[state]} label={t(`status.link.${state}`)} />;
}
