import { useCallback, useEffect, useState } from "react";

export const ROUTES = ["link", "logs", "audio", "keys", "settings", "actions"] as const;
export type AppRoute = (typeof ROUTES)[number];

function readRoute(): AppRoute {
  const raw = window.location.hash.replace(/^#\/?/, "").split("?")[0] ?? "";
  return (ROUTES as readonly string[]).includes(raw) ? (raw as AppRoute) : "link";
}

export function useHashRoute(): { route: AppRoute; navigate: (route: AppRoute) => void } {
  const [route, setRoute] = useState<AppRoute>(() => readRoute());

  useEffect(() => {
    const onChange = () => setRoute(readRoute());
    window.addEventListener("hashchange", onChange);
    if (!window.location.hash) {
      window.location.hash = "#/link";
    }
    return () => window.removeEventListener("hashchange", onChange);
  }, []);

  const navigate = useCallback((next: AppRoute) => {
    if (readRoute() !== next) window.location.hash = `#/${next}`;
    else setRoute(next);
  }, []);

  return { route, navigate };
}
