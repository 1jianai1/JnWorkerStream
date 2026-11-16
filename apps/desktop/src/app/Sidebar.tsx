import type { AppRoute } from "../lib/router";
import type { AppState } from "../lib/types";
import { useI18n } from "../lib/i18n";
import { Icon, type IconName } from "../components/Icon";

const groups: Array<{ labelKey: string; items: Array<{ route: AppRoute | null; labelKey: string; icon: IconName; badgeKey?: string; disabled?: boolean }> }> = [
  {
    labelKey: "nav.section.run",
    items: [
      { route: "link", labelKey: "nav.link", icon: "activity" },
      { route: "logs", labelKey: "nav.logs", icon: "terminal" },
      { route: "audio", labelKey: "nav.audio", icon: "wave" },
    ],
  },
  {
    labelKey: "nav.section.config",
    items: [
      { route: "keys", labelKey: "nav.keys", icon: "keyboard" },
      { route: "settings", labelKey: "nav.settings", icon: "settings" },
    ],
  },
  {
    labelKey: "nav.section.later",
    items: [
      { route: "actions", labelKey: "nav.actions", icon: "zap", badgeKey: "nav.laterBadge", disabled: true },
      { route: null, labelKey: "nav.ai", icon: "sparkles", badgeKey: "nav.laterBadgeAi", disabled: true },
    ],
  },
];

export function Sidebar({
  route,
  onNavigate,
  state,
}: {
  route: AppRoute;
  onNavigate: (route: AppRoute) => void;
  state: AppState;
}) {
  const { t } = useI18n();
  return (
    <nav className="sidebar" aria-label={t("app.name")}>
      {groups.map((group) => (
        <div className="sidebar-section" key={group.labelKey}>
          <span className="sidebar-label">{t(group.labelKey)}</span>
          {group.items.map((item) => (
            <button
              key={item.labelKey}
              type="button"
              className="nav-item"
              aria-current={item.route === route ? "page" : undefined}
              disabled={item.disabled}
              onClick={() => item.route && onNavigate(item.route)}
            >
              <span className="nav-icon">
                <Icon name={item.icon} size={17} />
              </span>
              <span className="nav-label">{t(item.labelKey)}</span>
              {item.badgeKey ? <span className="nav-badge">{t(item.badgeKey)}</span> : null}
            </button>
          ))}
        </div>
      ))}

      <div className="sidebar-foot">
        <span className="sidebar-foot-title">{t("link.detail.title")}</span>
        <div className="sidebar-foot-row">
          <span className="muted">{t("link.detail.state")}</span>
          <span>{t(`status.link.${state.link.state}`)}</span>
        </div>
        <div className="sidebar-foot-row">
          <span className="muted">{t("link.detail.kind")}</span>
          <span>{state.link.kind ? t(`link.control.${state.link.kind}`) : t("common.notAvailable")}</span>
        </div>
        <div className="sidebar-foot-row">
          <span className="muted">{t("link.detail.mtu")}</span>
          <span className="tabular">{state.link.mtu ?? t("common.notAvailable")}</span>
        </div>
        <div className="sidebar-foot-row">
          <span className="muted">{t("settings.about.version")}</span>
          <span className="tabular">{state.app.version}</span>
        </div>
      </div>
    </nav>
  );
}
