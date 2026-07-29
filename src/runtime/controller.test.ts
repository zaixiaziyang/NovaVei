import { describe, expect, it, vi } from "vitest";
import { PiRuntimeController } from "./controller";
import type {
  PiRunEvent,
  PiRunHandle,
  PiRunInput,
  PiRunResult,
  PiRuntimeTransport,
} from "./types";

class DeferredTransport implements PiRuntimeTransport {
  readonly runs: PiRunInput[] = [];
  readonly cancels: PiRunHandle[] = [];
  private listener: ((event: PiRunEvent) => void) | undefined;
  private readonly resolvers = new Map<string, (result: PiRunResult) => void>();

  subscribe(listener: (event: PiRunEvent) => void) {
    this.listener = listener;
    return () => {
      this.listener = undefined;
    };
  }

  run(input: PiRunInput) {
    this.runs.push(input);
    return new Promise<PiRunResult>((resolve) => {
      this.resolvers.set(input.requestId, resolve);
    });
  }

  cancel(handle: PiRunHandle) {
    this.cancels.push(handle);
    return Promise.resolve();
  }

  resolveRun(requestId: string) {
    const input = this.runs.find((run) => run.requestId === requestId);
    if (!input) throw new Error(`unknown run ${requestId}`);
    this.resolvers.get(requestId)?.({
      requestId,
      sessionId: input.sessionId,
      conversationId: input.conversationId ?? input.sessionId,
      turnId: `turn-${requestId}`,
    });
  }

  emit(event: PiRunEvent) {
    this.listener?.(event);
  }
}

describe("PiRuntimeController session isolation", () => {
  it("starts runs in different sessions without cancelling the first run", async () => {
    const transport = new DeferredTransport();
    const controller = new PiRuntimeController(transport);

    const first = controller.submit({ text: "first", sessionId: "session-a" });
    await vi.waitFor(() => expect(transport.runs).toHaveLength(1));

    const second = controller.submit({
      text: "second",
      sessionId: "session-b",
    });
    await vi.waitFor(() => expect(transport.runs).toHaveLength(2));

    expect(transport.cancels).toHaveLength(0);
    transport.resolveRun(transport.runs[0].requestId);
    transport.resolveRun(transport.runs[1].requestId);
    await Promise.all([first, second]);

    controller.dispose();
  });

  it("broadcasts terminal state for a background session", async () => {
    const transport = new DeferredTransport();
    const controller = new PiRuntimeController(transport);
    const lifecycle: string[] = [];

    controller.selectSession("session-b");
    const unsubscribe = controller.subscribeSessionState((sessionId, state) => {
      lifecycle.push(`${sessionId}:${state.status}`);
    });

    const run = controller.submit({
      text: "background",
      sessionId: "session-a",
    });
    await vi.waitFor(() => expect(transport.runs).toHaveLength(1));
    const [{ requestId }] = transport.runs;
    transport.resolveRun(requestId);
    await run;
    transport.emit({
      type: "done",
      requestId,
      sessionId: "session-a",
      conversationId: "session-a",
      turnId: `turn-${requestId}`,
      text: "ok",
    });

    await vi.waitFor(() => expect(lifecycle).toContain("session-a:completed"));
    expect(controller.getState().status).toBe("idle");

    unsubscribe();
    controller.dispose();
  });
});
