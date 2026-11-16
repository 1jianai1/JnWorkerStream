import { describe, expect, it } from "vitest";
import { LINK_TONE, LOG_TONE, VOICE_TONE, triggerAvailable } from "../src/lib/status";
import type { LinkState, LogLevel, VoicePhase } from "../src/lib/types";

describe("status mappings", () => {
  it("covers every link state with a semantic tone", () => {
    const states: LinkState[] = ["disconnected", "connecting", "connected", "reconnecting", "error"];
    expect(Object.keys(LINK_TONE).sort()).toEqual(states.sort());
    expect(LINK_TONE.connected).toBe("success");
    expect(LINK_TONE.error).toBe("danger");
    expect(LINK_TONE.reconnecting).toBe("warning");
  });

  it("covers every log level and voice phase", () => {
    const levels: LogLevel[] = ["trace", "debug", "info", "warn", "error"];
    const phases: VoicePhase[] = [
      "idle",
      "recording",
      "finalizing",
      "queued",
      "transcribing",
      "injecting",
      "done",
      "failed",
    ];
    expect(Object.keys(LOG_TONE).sort()).toEqual(levels.sort());
    expect(Object.keys(VOICE_TONE).sort()).toEqual(phases.sort());
    expect(LOG_TONE.error).toBe("danger");
    expect(VOICE_TONE.done).toBe("success");
    expect(VOICE_TONE.failed).toBe("danger");
  });

  it("only enables triggers for host-routed keys", () => {
    expect(triggerAvailable("host")).toBe(true);
    expect(triggerAvailable("deviceMacro")).toBe(false);
    expect(triggerAvailable("devicePassthrough")).toBe(false);
  });
});
