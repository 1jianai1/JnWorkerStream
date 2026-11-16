import { describe, expect, it } from "vitest";
import {
  formatBytes,
  formatDuration,
  formatRate,
  formatUptime,
  pad2,
} from "../src/lib/format";

describe("format helpers", () => {
  it("formats bytes with binary units", () => {
    expect(formatBytes("en-US", 0)).toBe("0 B");
    expect(formatBytes("en-US", 1024)).toBe("1 KB");
    expect(formatBytes("en-US", 1536)).toBe("1.5 KB");
    expect(formatBytes("en-US", 1024 * 1024)).toBe("1 MB");
  });

  it("formats rates and durations", () => {
    expect(formatRate("en-US", 0)).toBe("0 B/s");
    expect(formatRate("en-US", 2048)).toBe("2 KB/s");
    expect(formatDuration(0)).toBe("0 ms");
    expect(formatDuration(999)).toBe("999 ms");
    expect(formatDuration(1500)).toBe("1.50 s");
    expect(formatDuration(12_300)).toBe("12.3 s");
  });

  it("splits uptime and pads values", () => {
    expect(formatUptime(3723)).toEqual({ h: 1, m: 2, s: 3 });
    expect(formatUptime(-5)).toEqual({ h: 0, m: 0, s: 0 });
    expect(pad2(3)).toBe("03");
  });

  it("returns an em dash for non-finite input", () => {
    expect(formatBytes("en-US", Number.NaN)).toBe("—");
    expect(formatDuration(Number.NaN)).toBe("—");
  });
});
