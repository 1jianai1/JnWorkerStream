import { useCallback, useEffect } from "react";
import { I18nProvider } from "../lib/i18n";
import { ToastProvider } from "../lib/toast";
import type { Locale } from "../lib/i18n-core";
import { backend } from "../lib/ipc";
import { useAppState } from "../lib/store";
import { useHashRoute, type AppRoute } from "../lib/router";
import { AppShell } from "./AppShell";
import { LinkPage } from "../features/link/LinkPage";
import { LogsPage } from "../features/logs/LogsPage";
import { AudioPage } from "../features/audio/AudioPage";
import { KeysPage } from "../features/keys/KeysPage";
import { SettingsPage } from "../features/settings/SettingsPage";
import { PlaceholderPage } from "../features/PlaceholderPage";

export function App() {
  const state = useAppState();
  const { route, navigate } = useHashRoute();

  const onLocaleChange = useCallback((locale: Locale) => {
    void backend.updateSettings({ ui: { locale } });
  }, []);

  useEffect(() => {
    document.documentElement.lang = state.settings.ui.locale;
    document.title = state.settings.ui.locale === "zh-CN"
      ? "JnWorkerStream · 桌面控制台"
      : "JnWorkerStream · Desktop Console";
  }, [state.settings.ui.locale]);

  const page = (() => {
    switch (route) {
      case "link":
        return <LinkPage />;
      case "logs":
        return <LogsPage />;
      case "audio":
        return <AudioPage />;
      case "keys":
        return <KeysPage />;
      case "settings":
        return <SettingsPage />;
      case "actions":
        return <PlaceholderPage kind="actions" onBack={() => navigate("link")} />;
      default:
        return <LinkPage />;
    }
  })();

  return (
    <I18nProvider locale={state.settings.ui.locale} onLocaleChange={onLocaleChange}>
      <ToastProvider>
        <AppShell state={state} route={route} onNavigate={navigate}>
          {page}
        </AppShell>
      </ToastProvider>
    </I18nProvider>
  );
}

export type { AppRoute };
