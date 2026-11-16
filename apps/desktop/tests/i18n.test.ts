import { describe, expect, it } from "vitest";
import zhCN from "../src/locales/zh-CN.json";
import enUS from "../src/locales/en-US.json";
import type { MessageDict } from "../src/lib/i18n-core";
import { diffKeys, flattenKeys, interpolate, lookup, normalizeLocale } from "../src/lib/i18n-core";

describe("i18n resources", () => {
  it("keeps zh-CN and en-US key sets identical", () => {
    const diff = diffKeys(zhCN as MessageDict, enUS as MessageDict);
    expect(diff.missing).toEqual([]);
    expect(diff.extra).toEqual([]);
  });

  it("has no empty or placeholder-only strings", () => {
    for (const [name, dict] of [["zh-CN", zhCN], ["en-US", enUS]] as const) {
      for (const key of flattenKeys(dict as MessageDict)) {
        const value = lookup(dict as MessageDict, key);
        expect(value, `${name}:${key}`).toBeTruthy();
        expect(value?.trim().length, `${name}:${key}`).toBeGreaterThan(0);
        expect(value, `${name}:${key}`).not.toContain("[TODO]");
      }
    }
  });

  it("interpolates named parameters and leaves unknown placeholders intact", () => {
    expect(interpolate("K{key} · {count} 项", { key: 7, count: 3 })).toBe("K7 · 3 项");
    expect(interpolate("{a}-{b}", { a: "x" })).toBe("x-{b}");
  });

  it("normalises unsupported locales to zh-CN", () => {
    expect(normalizeLocale("en-US")).toBe("en-US");
    expect(normalizeLocale("fr-FR")).toBe("zh-CN");
    expect(normalizeLocale(undefined)).toBe("zh-CN");
  });
});
