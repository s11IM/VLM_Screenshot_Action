export type ToolOutcome = "succeeded" | "blocked" | "failed";

/**
 * How many steps in a row may fail to execute before the round gives up.
 *
 * A single failure is normal and self-healing: the caller takes a fresh
 * screenshot after it, so the next attempt starts from an updated frame and
 * foreground window. Reaching this limit therefore means the model keeps
 * producing steps that cannot run at all, and the loop would otherwise repeat
 * forever, since it has no other bound than the per-request timeout.
 */
export const TOOL_FAILURE_LIMIT = 3;

export function nextToolFailureStreak(streak: number, outcome: ToolOutcome): number {
  return outcome === "succeeded" ? 0 : streak + 1;
}

export function toolFailureLimitReached(streak: number): boolean {
  return streak >= TOOL_FAILURE_LIMIT;
}
