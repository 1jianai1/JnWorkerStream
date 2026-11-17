import { describe, expect, it, vi } from "vitest";

/**
 * 后端选择逻辑（`src/lib/ipc.ts`）的回归测试。
 *
 * 真实 Tauri 里 `window.__TAURI_INTERNALS__` 存在；纯浏览器（dev/e2e/截图）没有。
 * jsdom 下没有该属性，所以默认必须走 Mock，否则 e2e 与截图基线会全挂。
 */
async function load(query: string) {
  window.history.replaceState({}, "", `/${query}`);
  vi.resetModules();
  return import("../src/lib/ipc");
}

describe("backend selection", () => {
  it("plain browser falls back to the mock backend", async () => {
    const mod = await load("");
    expect(mod.runtime.isMock).toBe(true);
    expect(mod.backend.constructor.name).toBe("MockBackend");
  });

  it("scenario query keeps the mock (screenshot/e2e baseline)", async () => {
    const mod = await load("?scenario=connected");
    expect(mod.runtime.isMock).toBe(true);
    expect(mod.backend.constructor.name).toBe("MockBackend");
  });

  it("backend=tauri selects the real backend", async () => {
    const mod = await load("?backend=tauri");
    expect(mod.runtime.isMock).toBe(false);
    expect(mod.backend.constructor.name).toBe("TauriBackend");
    // 初始快照还没回来时也必须是可渲染的完整状态。
    const state = mod.backend.getState();
    expect(state.link.state).toBe("disconnected");
    expect(state.voice.phase).toBe("idle");
    expect(Array.isArray(state.logs)).toBe(true);
  });

  it("backend=mock wins over Tauri detection", async () => {
    const mod = await load("?backend=mock");
    expect(mod.runtime.isMock).toBe(true);
  });
});
