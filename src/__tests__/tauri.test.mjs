import assert from "node:assert/strict";
import test from "node:test";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { captureRegion, executeInputAction } from "../tauri.ts";

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
  } finally {
    clearMocks();
    delete globalThis.window;
  }
});
