import { Button, Card, EmptyState, PageHeader } from "../components/primitives";
import { useI18n } from "../lib/i18n";

export function PlaceholderPage({ kind, onBack }: { kind: "actions" | "ai"; onBack: () => void }) {
  const { t } = useI18n();
  return (
    <div className="page">
      <PageHeader title={t(`${kind}.placeholderTitle`)} subtitle={t(`${kind}.placeholderText`)} />
      <div className="page-body">
        <Card>
          <EmptyState
            icon={kind === "actions" ? "zap" : "sparkles"}
            title={t(`${kind}.placeholderTitle`)}
            text={t(`${kind}.placeholderText`)}
            actions={<Button variant="secondary" onClick={onBack}>{t(`${kind}.back`)}</Button>}
          />
        </Card>
      </div>
    </div>
  );
}
