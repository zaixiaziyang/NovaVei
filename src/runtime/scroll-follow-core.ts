/**
 * Pure, DOM-free follow-scroll state machine for the transcript.
 *
 * The web shell must keep a streamed transcript pinned only while the reader
 * is deliberately watching the latest content. The tricky part is deciding
 * what "deliberately" means: a passive streamed growth should never yank a
 * reader back down after they scroll into history, and returning to the
 * bottom must survive WebView2 smooth-scroll latency and fractional device
 * pixel rounding without flickering.
 *
 * The rules here are intentionally input-driven. Only an explicit reader
 * gesture (scroll-up, track drag, or an explicit history navigation key)
 * detaches follow; reattaching requires several independent gates so an
 * incidental near-bottom position does not snap the reader into a stream they
 * are not watching.
 */

/** Detach on scroll-up / drag only when the reader has moved past this noise floor. */
const DETACH_MIN_DELTA_PX = 2;

/**
 * Reattach zone: the reader is considered "back at the latest content" when
 * the viewport bottom sits within REATTACH_ZONE_PX of the scrollable bottom.
 * Kept just under LiveAgent's 192 px heuristic so a browser-level smooth
 * scroll end lands inside the zone.
 */
export const REATTACH_ZONE_PX = 198;

export type FollowEvent =
  | {
      type: "scroll";
      deltaY: number;
      scrollTop: number;
      clientHeight: number;
      scrollHeight: number;
    }
  | { type: "pointer"; phase: "down" | "move" | "up"; deltaY?: number }
  | { type: "history-key"; active: boolean }
  | { type: "content-grown" };

export type FollowState = {
  trailing: boolean;
  /** Track drag / touch held down gates reattach until the finger is released. */
  gestureLocked: boolean;
};

export function createScrollFollowState(): FollowState {
  return {
    trailing: true,
    gestureLocked: false,
  };
}

export interface ScrollFollowDecision {
  follow: boolean;
  reason: "follow" | "detach" | "reattach";
}

const KEEP: ScrollFollowDecision = { follow: true, reason: "follow" };
const DETACH: ScrollFollowDecision = { follow: false, reason: "detach" };
const REATTACH: ScrollFollowDecision = { follow: true, reason: "reattach" };
const STILL_DETACHED: ScrollFollowDecision = {
  follow: false,
  reason: "detach",
};

/** DPR / sub-pixel rounding guard: always compare whole pixels. */
function rounded(value: number) {
  return Math.round(value);
}

export function isWithinZone(
  scrollTop: number,
  clientHeight: number,
  scrollHeight: number,
  zone: number,
) {
  return (
    rounded(scrollHeight) - (rounded(scrollTop) + rounded(clientHeight)) <= zone
  );
}

/**
 * Fold one follow event into the state. Returns the next state plus the
 * follow decision to apply to the viewport.
 */
export function scrollFollow(
  st: FollowState,
  event: FollowEvent,
): { state: FollowState; decision: ScrollFollowDecision } {
  switch (event.type) {
    case "scroll": {
      const nearBottom = isWithinReattachZone(
        event.scrollTop,
        event.clientHeight,
        event.scrollHeight,
      );
      const explicitDetach = event.deltaY < -DETACH_MIN_DELTA_PX && !nearBottom;
      if (explicitDetach && st.trailing) {
        return {
          state: { trailing: false, gestureLocked: st.gestureLocked },
          decision: DETACH,
        };
      }
      if (!st.trailing && !st.gestureLocked && nearBottom) {
        return {
          state: { trailing: true, gestureLocked: st.gestureLocked },
          decision: REATTACH,
        };
      }
      return {
        state: st,
        decision: st.trailing ? KEEP : STILL_DETACHED,
      };
    }
    case "pointer": {
      if (event.phase === "down") {
        const next = { ...st, gestureLocked: true };
        return { state: next, decision: next.trailing ? KEEP : STILL_DETACHED };
      }
      if (event.phase === "move") {
        const delta = event.deltaY ?? 0;
        const detaching = delta < -DETACH_MIN_DELTA_PX && st.trailing;
        const next = detaching ? { ...st, trailing: false } : st;
        return {
          state: next,
          decision: next.trailing ? KEEP : STILL_DETACHED,
        };
      }
      // "up": finger released. Clear the lock; a following scroll event's
      // near-bottom gate decides reattach (so wheel momentum settles first).
      return {
        state: { ...st, gestureLocked: false },
        decision: st.trailing ? KEEP : STILL_DETACHED,
      };
    }
    case "history-key": {
      if (event.active) {
        return {
          state: { trailing: true, gestureLocked: st.gestureLocked },
          decision: REATTACH,
        };
      }
      return { state: st, decision: STILL_DETACHED };
    }
    case "content-grown": {
      // Passive stream growth never detaches and, while trailing, keeps
      // following; while detached it stays detached.
      return { state: st, decision: st.trailing ? KEEP : STILL_DETACHED };
    }
  }
}

function isWithinReattachZone(
  scrollTop: number,
  clientHeight: number,
  scrollHeight: number,
) {
  return isWithinZone(scrollTop, clientHeight, scrollHeight, REATTACH_ZONE_PX);
}
