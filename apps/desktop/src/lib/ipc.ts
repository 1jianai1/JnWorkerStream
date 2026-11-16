import type { Backend } from "./backend";
import { MockBackend } from "../mock/backend";
import { isScenario } from "../mock/seed";

const params = new URLSearchParams(typeof window === "undefined" ? "" : window.location.search);
const requestedScenario = params.get("scenario");
const scenario = isScenario(requestedScenario) ? requestedScenario : "default";

/**
 * P1 ships the mock backend only. Components import `backend` from this module;
 * they must never import `src/mock/**` directly (enforced by ESLint).
 */
export const backend: Backend = new MockBackend(scenario);

export const runtime = {
  scenario,
  isMock: true,
  rendererOverride: params.get("renderer"),
  backendOverride: params.get("backend"),
  forcedLocale: params.get("lang"),
} as const;
