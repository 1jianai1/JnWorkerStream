import type { Backend } from "./backend";
import { MockBackend } from "../mock/backend";
import { isScenario } from "../mock/seed";
import { TauriBackend } from "./tauri-backend";

const params = new URLSearchParams(typeof window === "undefined" ? "" : window.location.search);
const requestedScenario = params.get("scenario");
const scenario = isScenario(requestedScenario) ? requestedScenario : "default";

/** Tauri 2 会在 window 上挂 `__TAURI_INTERNALS__`；纯浏览器（dev/e2e）没有。 */
function inTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/**
 * 后端选择：
 *
 * 1. `?backend=tauri` 强制真实后端（调试用）；
 * 2. `?backend=mock` 或任何 `?scenario=` 强制 Mock（e2e/截图基线靠这个）；
 * 3. 否则在 Tauri 里用真实后端，在浏览器里用 Mock。
 *
 * 组件只从本模块拿 `backend`，不得直接 import `src/mock/**`（ESLint 约束）。
 */
function selectBackend(): { backend: Backend; isMock: boolean } {
  const override = params.get("backend");
  if (override === "mock" || requestedScenario !== null) {
    return { backend: new MockBackend(scenario), isMock: true };
  }
  if (override === "tauri") {
    return { backend: new TauriBackend(), isMock: false };
  }
  return inTauri()
    ? { backend: new TauriBackend(), isMock: false }
    : { backend: new MockBackend(scenario), isMock: true };
}

const selected = selectBackend();

export const backend: Backend = selected.backend;

export const runtime = {
  scenario,
  isMock: selected.isMock,
  rendererOverride: params.get("renderer"),
  backendOverride: params.get("backend"),
  forcedLocale: params.get("lang"),
} as const;
