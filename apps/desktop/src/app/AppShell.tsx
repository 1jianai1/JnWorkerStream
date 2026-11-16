import type { ReactNode } from "react";
import type { AppState } from "../lib/types";
import type { AppRoute } from "../lib/router";
import { useI18n } from "../lib/i18n";
import { LinkStatePill } from "./LinkStatePill";
import { Sidebar } from "./Sidebar";
import { StatusBar } from "./StatusBar";
import { Icon } from "../components/Icon";
import { SplitHandle, useResizablePanel } from "../components/ResizablePanel";

export function AppShell({
  state,
  route,
  onNavigate,
  children,
}: {
  state: AppState;
  route: AppRoute;
  onNavigate: (route: AppRoute) => void;
  children: ReactNode;
}) {
  const { t, locale, setLocale } = useI18n();
  const sidebar = useResizablePanel({
    storageKey: "sidebar",
    initial: () => (window.innerWidth >= 1600 ? 300 : 260),
    min: 208,
    max: 360,
    edge: "right",
    label: t("layout.resizeSidebar"),
    cssVar: "--sidebar-width",
  });
  return (
    <div className="app-shell" style={sidebar.style}>
      <header className="topbar">
        <div className="brand">
          <span className="brand-mark" aria-hidden="true">JN</span>
          <div className="stack" style={{ gap: 0 }}>
            <span className="brand-name">{t("app.name")}</span>
            <span className="brand-sub">{t("app.subtitle")}</span>
          </div>
        </div>
        <span className="tag tag--warning" title={t("common.demoData")}>{t("app.p1")}</span>
        <div className="topbar-spacer" />
        <div className="topbar-meta">
          <button
            type="button"
            className="topbar-link"
            onClick={() => onNavigate("link")}
            title={t("link.title")}
          >
            <LinkStatePill state={state.link.state} />
          </button>
          <span className="statusbar-sep" aria-hidden="true" />
          <span className="topbar-label">{t("topbar.language")}</span>
          <div className="segmented" role="group" aria-label={t("topbar.language")}>
            <button
              type="button"
              className="segmented__item"
              aria-pressed={locale === "zh-CN"}
              onClick={() => setLocale("zh-CN")}
            >
              {t("topbar.localeZh")}
            </button>
            <button
              type="button"
              className="segmented__item"
              aria-pressed={locale === "en-US"}
              onClick={() => setLocale("en-US")}
            >
              {t("topbar.localeEn")}
            </button>
          </div>
          <span className="statusbar-sep" aria-hidden="true" />
          <span className="topbar-label topbar-device" title={state.keys.device_online ? t("topbar.deviceOnline") : t("topbar.deviceOffline")}>
            <Icon name={state.keys.device_online ? "check" : "alertTriangle"} size={13} />
            {state.keys.device_online ? t("topbar.deviceOnline") : t("topbar.deviceOffline")}
          </span>
        </div>
      </header>
      <div className="sidebar-slot">
        <Sidebar route={route} onNavigate={onNavigate} state={state} />
        <SplitHandle edge="right" handleProps={sidebar.handleProps} />
      </div>
      <main className="main-area">{children}</main>
      <StatusBar state={state} />
      <div className="size-guard" role="alertdialog" aria-label={t("common.windowTooSmall.title")}>
        <div className="size-guard__panel">
          <h2 className="page-title" style={{ marginBottom: 8 }}>{t("common.windowTooSmall.title")}</h2>
          <p className="page-subtitle">{t("common.windowTooSmall.text")}</p>
        </div>
      </div>
    </div>
  );
}
