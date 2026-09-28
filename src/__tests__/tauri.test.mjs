import assert from "node:assert/strict";
import test from "node:test";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { captureRegion, executeInputAction, observeRegion } from "../tauri.ts";

test("carries the captured frame token into the input command", async () => {
  globalThis.window = {};
  const region = { x: 0, y: 0, width: 800, height: 600 };
  const capture = { dataUrl: "data:image/png;base64,test", frameId: "native-frame" };
  const calls = [];
  mockIPC((command, payload) => {
    calls.push({ command, payload });
    return command === "capture_region" ? capture : "dispatched";
  });
  try {
    const image = await captureRegion(region, { captureKind: "manual" });
    assert.deepEqual(image, capture);
    await executeInputAction(region, { type: "keyboard_press", keys: ["A"] }, "op", "round", 1, "call", image.frameId);
    assert.equal(calls[1].command, "execute_input_action");
    assert.equal(calls[1].payload.frameId, "native-frame");
    assert.deepEqual(calls[1].payload.region, region);
    await executeInputAction(region, { type: "keyboard_type", text: "x" }, "op", "round", 2, "legacy");
    assert.equal(calls[2].payload.frameId, undefined);
    const observation = {
      dataUrl: capture.dataUrl,
      frameId: "observed-frame",
      outcome: "deadline",
      waitedMs: 7000,
      samples: 4,
      trigger: null,
    };
    calls.length = 0;
    mockIPC((command, payload) => {
      calls.push({ command, payload });
      return observation;
    });
    assert.deepEqual(await observeRegion(region, {
      operationId: "op",
      roundId: "round",
      captureKind: "operation-result",
      toolStep: 3,
      deadlineMs: 20000,
      probeEnabled: true,
      probeDelayMs: 3250,
      markerX: 10,
      markerY: 20,
    }), observation);
    assert.equal(calls[0].command, "observe_region");
    assert.deepEqual(calls[0].payload, {
      region,
      operationId: "op",
      roundId: "round",
      captureKind: "operation-result",
      toolStep: 3,
      deadlineMs: 20000,
      probeEnabled: true,
      probeDelayMs: 3250,
      markerX: 10,
      markerY: 20,
    });
  } finally {
    clearMocks();
    delete globalThis.window;
  }
});
