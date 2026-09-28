import type { RegionObservation, Settings } from "./types.ts";

export function observationProbeDelay(actionOutputAtMs: number, nowMs: number): number {
  // Sampling calibrates during this window; only early wake is locked.
  return Math.max(0, Math.ceil(4000 - (nowMs - actionOutputAtMs)));
}

export function observationWindow(
  tool: string,
  remainingMs: number | null,
  settings: Pick<Settings, "operationCaptureDelay" | "earlyProbeEnabled">,
) {
  if (tool === "wait" && remainingMs !== null) {
    return { deadlineMs: remainingMs, probeEnabled: false };
  }
  return {
    deadlineMs: Math.max(0.5, settings.operationCaptureDelay) * 1000,
    probeEnabled: settings.earlyProbeEnabled,
  };
}

export function pausedObservationRemainder(
  deadlineMs: number,
  observed: Pick<RegionObservation, "outcome" | "waitedMs">,
): number | null {
  return observed.outcome === "early_wake"
    ? Math.max(0, deadlineMs - observed.waitedMs)
    : null;
}
