import { useSyncExternalStore } from "react";
import { backend } from "./ipc";
import type { AppState } from "./types";

export function useAppState(): AppState {
  return useSyncExternalStore(backend.subscribe, backend.getState, backend.getState);
}
