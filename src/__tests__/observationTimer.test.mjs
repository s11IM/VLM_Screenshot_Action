import assert from "node:assert/strict";
import test from "node:test";
import { observationProbeDelay, observationWindow, pausedObservationRemainder } from "../observationTimer.ts";

const settings = { operationCaptureDelay: 8, earlyProbeEnabled: true };

test("probing is suppressed for four seconds from model action output, not action completion", () => {
  assert.equal(observationProbeDelay(1000, 1000), 4000);
  assert.equal(observationProbeDelay(1000, 1750), 3250);
  assert.equal(observationProbeDelay(1000, 4999), 1);
  assert.equal(observationProbeDelay(1000, 4999.5), 1);
  assert.equal(observationProbeDelay(1000, 5000), 0);
  assert.equal(observationProbeDelay(1000, 6500), 0);
  assert.equal(observationProbeDelay(6500, 6500), 4000);
});

test("early wake freezes the remainder and wait resumes it without probing", () => {
  const remaining = pausedObservationRemainder(8000, { outcome: "early_wake", waitedMs: 2820 });
  assert.equal(remaining, 5180);
  assert.deepEqual(observationWindow("wait", remaining, settings), {
    deadlineMs: 5180, probeEnabled: false,
  });
  // Planning a failed/rejected call cannot mutate the paused remainder.
  observationWindow("click", remaining, settings);
  assert.deepEqual(observationWindow("wait", remaining, settings), {
    deadlineMs: 5180, probeEnabled: false,
  });
});

test("actions reset the timer and wait after deadline uses the same setting", () => {
  for (const tool of ["click", "drag", "hover", "keyboard_type", "keyboard_press"]) {
    assert.deepEqual(observationWindow(tool, 100, settings), {
      deadlineMs: 8000, probeEnabled: true,
    });
  }
  const remaining = pausedObservationRemainder(8000, { outcome: "deadline", waitedMs: 8050 });
  assert.equal(remaining, null);
  assert.deepEqual(observationWindow("wait", remaining, settings), {
    deadlineMs: 8000, probeEnabled: true,
  });
});

test("resuming never rounds short or exhausted remainders up to 500ms", () => {
  for (const [waitedMs, expected] of [[7601, 399], [7999, 1], [8000, 0], [8050, 0]]) {
    const remaining = pausedObservationRemainder(8000, { outcome: "early_wake", waitedMs });
    assert.equal(remaining, expected);
    assert.deepEqual(observationWindow("wait", remaining, settings), {
      deadlineMs: expected, probeEnabled: false,
    });
  }
});

test("disabling early probing keeps the configured timer", () => {
  assert.deepEqual(observationWindow("wait", null, { ...settings, earlyProbeEnabled: false }), {
    deadlineMs: 8000, probeEnabled: false,
  });
});

test("an exhausted resume returns to a full window after its deadline capture", () => {
  const resumed = observationWindow("wait", 0, settings);
  assert.equal(resumed.deadlineMs, 0);
  const remaining = pausedObservationRemainder(resumed.deadlineMs, {
    outcome: "deadline", waitedMs: 150,
  });
  assert.equal(remaining, null);
  assert.deepEqual(observationWindow("wait", remaining, settings), {
    deadlineMs: 8000, probeEnabled: true,
  });
});
