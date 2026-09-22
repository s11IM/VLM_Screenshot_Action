import { invoke } from "@tauri-apps/api/core";
import type { CapturedImage, ChatCompletionResponse, Region } from "./types";

export const isTauri = () => "__TAURI_INTERNALS__" in window;

export function logClientEvent(
  event: string,
  data: Record<string, unknown> = {},
  level: "debug" | "info" | "warn" | "error" = "info",
) {
  if (!isTauri()) return;
  void invoke("client_log", { level, event, data }).catch(() => undefined);
}

export async function selectRegion(): Promise<Region> {
  return invoke<Region>("select_region");
}

export async function enterMiniMode(): Promise<void> {
  if (!isTauri()) return;
  await invoke("enter_mini_mode");
}

export async function exitMiniMode(): Promise<void> {
  if (!isTauri()) return;
  await invoke("exit_mini_mode");
}

export async function captureRegion(
  region: Region,
  context: {
    operationId?: string;
    roundId?: string;
    captureKind?: string;
    toolStep?: number;
    markerX?: number;
    markerY?: number;
    markerFromX?: number;
    markerFromY?: number;
  } = {},
): Promise<CapturedImage> {
  return invoke<CapturedImage>("capture_region", { region, ...context });
}

export async function beginOperation(
  operationId: string,
  roundId: string,
  trigger: string,
): Promise<void> {
  if (!isTauri()) return;
  await invoke("begin_operation", { operationId, roundId, trigger });
}

export async function cancelOperation(operationId: string, roundId: string): Promise<void> {
  if (!isTauri()) return;
  await invoke("cancel_operation", { operationId, roundId });
}

export async function finishOperation(
  operationId: string,
  roundId: string,
  outcome: string,
): Promise<void> {
  if (!isTauri()) return;
  await invoke("finish_operation", { operationId, roundId, outcome });
}

export async function executeInputAction(
  region: Region,
  action: Record<string, unknown>,
  operationId: string,
  roundId: string,
  toolStep: number,
  toolCallId: string,
  frameId?: string,
): Promise<string> {
  return invoke<string>("execute_input_action", {
    region,
    action,
    operationId,
    roundId,
    toolStep,
    toolCallId,
    frameId,
  });
}

export async function requestChatCompletion(payload: {
  apiUrl: string;
  apiKey: string;
  body: Record<string, unknown>;
  operationId: string;
  roundId: string;
  requestIndex: number;
  attempt: number;
  toolsAllowed: boolean;
  timeoutSeconds: number;
}): Promise<ChatCompletionResponse> {
  return invoke<ChatCompletionResponse>("request_chat_completion", payload);
}
