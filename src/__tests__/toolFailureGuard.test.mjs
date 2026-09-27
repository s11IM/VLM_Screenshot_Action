import assert from "node:assert/strict";
import test from "node:test";
import {
  nextToolFailureStreak,
  toolFailureLimitReached,
  TOOL_FAILURE_LIMIT,
} from "../toolFailureGuard.ts";

test("a successful step clears the streak, including wait and end_round", () => {
  let streak = 0;
  streak = nextToolFailureStreak(streak, "blocked");
  streak = nextToolFailureStreak(streak, "failed");
  assert.equal(streak, 2);
  // wait and end_round are reported as succeeded, so they must reset.
  streak = nextToolFailureStreak(streak, "succeeded");
  assert.equal(streak, 0);
  assert.equal(toolFailureLimitReached(streak), false);
});

test("blocked and failed steps count toward the same limit", () => {
  assert.equal(nextToolFailureStreak(0, "blocked"), 1);
  assert.equal(nextToolFailureStreak(0, "failed"), 1);
  assert.equal(nextToolFailureStreak(1, "blocked"), 2);
  assert.equal(nextToolFailureStreak(1, "failed"), 2);
});

test("the limit needs a full run of consecutive failures", () => {
  let streak = 0;
  for (let step = 0; step < TOOL_FAILURE_LIMIT - 1; step += 1) {
    streak = nextToolFailureStreak(streak, "failed");
  }
  assert.equal(toolFailureLimitReached(streak), false);

  streak = nextToolFailureStreak(streak, "failed");
  assert.equal(toolFailureLimitReached(streak), true);
});

test("one success in the middle prevents the abort", () => {
  let streak = 0;
  for (const outcome of ["failed", "blocked", "succeeded", "failed", "blocked"]) {
    streak = nextToolFailureStreak(streak, outcome);
  }
  assert.equal(streak, 2);
  assert.equal(toolFailureLimitReached(streak), false);
});

test("an already reached limit stays reached until a success resets it", () => {
  assert.equal(toolFailureLimitReached(TOOL_FAILURE_LIMIT), true);
  assert.equal(toolFailureLimitReached(TOOL_FAILURE_LIMIT + 5), true);
});
