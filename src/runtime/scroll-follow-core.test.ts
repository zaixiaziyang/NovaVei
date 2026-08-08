import { describe, expect, it } from "vitest";
import {
  createScrollFollowState,
  isWithinZone,
  REATTACH_ZONE_PX,
  scrollFollow,
  type ScrollFollowDecision,
  type FollowEvent,
} from "./scroll-follow-core";

function follow(events: FollowEvent[]): ScrollFollowDecision[] {
  let st = createScrollFollowState();
  return events.map((event) => {
    const out = scrollFollow(st, event);
    st = out.state;
    return out.decision;
  });
}

describe("scroll-follow-core", () => {
  it("starts trailing", () => {
    const st = createScrollFollowState();
    expect(st.trailing).toBe(true);
    expect(st.gestureLocked).toBe(false);
  });

  it("stays following on passive content growth", () => {
    const decisions = follow([{ type: "content-grown" }]);
    expect(decisions).toEqual([{ follow: true, reason: "follow" }]);
  });

  it("detaches only on explicit scroll-up, not on near-bottom scrolls", () => {
    const scrollTop = 500;
    const clientHeight = 600;
    const scrollHeight = 1000; // within reattach zone (1000-1100<=198)
    const decisions = follow([
      {
        type: "scroll",
        deltaY: -50,
        scrollTop,
        clientHeight,
        scrollHeight,
      },
      {
        type: "scroll",
        deltaY: 50,
        scrollTop: 500,
        clientHeight: 600,
        scrollHeight: 1000,
      },
    ]);
    // First: near-bottom already, so no detach.
    expect(decisions[0].follow).toBe(true);
    // Second tiniest scroll stays following since not detached.
    expect(decisions[1].follow).toBe(true);
  });

  it("detaches on scroll-up when away from the bottom", () => {
    const scrollHeight = 4000;
    const clientHeight = 600;
    const scrollTop = 2000; // bottom gap = 1400 > 198
    const decisions = follow([
      {
        type: "scroll",
        deltaY: -200,
        scrollTop,
        clientHeight,
        scrollHeight,
      },
    ]);
    expect(decisions).toEqual([{ follow: false, reason: "detach" }]);
  });

  it("stays detached while the reader is mid-history", () => {
    const base = {
      type: "scroll" as const,
      deltaY: -10,
      scrollTop: 1500,
      clientHeight: 600,
      scrollHeight: 4000,
    };
    const decisions = follow([
      { ...base, deltaY: -200 },
      { type: "content-grown" },
    ]);
    expect(decisions[0].reason).toBe("detach");
    // Passive growth must not re-yank the reader.
    expect(decisions[1]).toEqual({
      follow: false,
      reason: "detach",
    });
  });

  it("reattaches only when near-bottom AND gesture released", () => {
    const events: FollowEvent[] = [];
    // 1. detach
    events.push({
      type: "scroll",
      deltaY: -300,
      scrollTop: 2000,
      clientHeight: 600,
      scrollHeight: 4000,
    });
    // 2. gesture down
    events.push({ type: "pointer", phase: "down" });
    // 3. scroll near-bottom while gesture locked → must NOT reattach yet
    events.push({
      type: "scroll",
      deltaY: 10,
      scrollTop: 4000 - 600 - 10, // within 198 zone
      clientHeight: 600,
      scrollHeight: 4000,
    });
    // 4. release
    events.push({ type: "pointer", phase: "up" });
    // 5. reattach only after release with a fill scroll
    events.push({
      type: "scroll",
      deltaY: 10,
      scrollTop: 4000 - 600 - 5,
      clientHeight: 600,
      scrollHeight: 4000,
    });

    const decisions = follow(events);
    expect(decisions[0].reason).toBe("detach");
    expect(decisions[2].reason).toBe("detach"); // locked → still detached
    expect(decisions[3].reason).toBe("detach"); // release alone not reattach
    expect(decisions[4]).toEqual({ follow: true, reason: "reattach" });
  });

  it("reattaches on near-bottom scroll when not gesture-locked", () => {
    const decisions = follow([
      {
        type: "scroll",
        deltaY: -200,
        scrollTop: 2000,
        clientHeight: 600,
        scrollHeight: 4000,
      },
      {
        type: "scroll",
        deltaY: 10,
        scrollTop: 4000 - 600 - 5,
        clientHeight: 600,
        scrollHeight: 4000,
      },
    ]);
    expect(decisions[0].reason).toBe("detach");
    expect(decisions[1]).toEqual({ follow: true, reason: "reattach" });
  });

  it("history-key forces reattach/detach", () => {
    const events: FollowEvent[] = [
      {
        type: "scroll",
        deltaY: -200,
        scrollTop: 2000,
        clientHeight: 600,
        scrollHeight: 4000,
      },
      { type: "history-key", active: true },
      { type: "history-key", active: false },
    ];
    const decisions = follow(events);
    expect(decisions[0].reason).toBe("detach");
    expect(decisions[1]).toEqual({ follow: true, reason: "reattach" });
    expect(decisions[2].reason).toBe("detach");
  });

  it("isWithinZone rounds as whole pixels", () => {
    // scrollTop 3399.7 => rounded 3400, bottom gap = 4000-(3400+600)=0
    expect(isWithinZone(3399.7, 600.4, 4000.0, REATTACH_ZONE_PX)).toBe(true);
    expect(isWithinZone(2000, 600, 4000, REATTACH_ZONE_PX)).toBe(false);
  });
});
