import type { ToolCall } from "./types";

export const COMPUTER_TOOL_COORDINATE_MODE = "frame-pixels" as const;

type FrameSize = {
  width: number;
  height: number;
};

type AdapterContext = {
  offeredToolNames: ReadonlySet<string>;
  frameSize: FrameSize | null;
};

export type ComputerToolAdapterEvent = {
  callId: string;
  action: string | null;
  outcome: "adapted" | "rejected";
  adaptedTool?: string;
  reason?: string;
  coordinateMode?: typeof COMPUTER_TOOL_COORDINATE_MODE;
  sourceCoordinates?: number[][];
  normalizedCoordinates?: number[][];
};

type AdaptedCall = {
  call: ToolCall;
  event: ComputerToolAdapterEvent;
};

const adapterAnalysis = (action: string, tool: string) =>
  `Compatibility adapter mapped upstream computer.${action} to ${tool}.`;

const adaptedCall = (
  original: ToolCall,
  action: string,
  tool: ToolCall["function"]["name"],
  args: Record<string, unknown>,
  coordinates: { source: number[][]; normalized: number[][] } | null = null,
): AdaptedCall => ({
  call: {
    ...original,
    function: {
      name: tool,
      arguments: JSON.stringify(args),
    },
  },
  event: {
    callId: original.id,
    action,
    outcome: "adapted",
    adaptedTool: tool,
    coordinateMode: coordinates ? COMPUTER_TOOL_COORDINATE_MODE : undefined,
    sourceCoordinates: coordinates?.source,
    normalizedCoordinates: coordinates?.normalized,
  },
});

const rejectedEvent = (
  call: ToolCall,
  action: string | null,
  reason: string,
): ComputerToolAdapterEvent => ({
  callId: call.id,
  action,
  outcome: "rejected",
  reason,
});

const parseArguments = (call: ToolCall): Record<string, unknown> => {
  const parsed = JSON.parse(call.function.arguments) as unknown;
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error("computer arguments are not a JSON object");
  }
  return parsed as Record<string, unknown>;
};

const finiteNumber = (value: unknown, label: string) => {
  const scalar = typeof value === "string" && value.trim() ? Number(value) : value;
  if (typeof scalar !== "number" || !Number.isFinite(scalar)) {
    throw new Error(`${label} must be a finite number`);
  }
  return scalar;
};

const normalizedCoordinate = (
  value: unknown,
  label: string,
  frameSize: FrameSize | null,
): { source: number[]; normalized: number[] } => {
  if (!frameSize || frameSize.width <= 1 || frameSize.height <= 1) {
    throw new Error(`${label} requires current frame dimensions`);
  }
  if (!Array.isArray(value) || value.length !== 2) {
    throw new Error(`${label} must be [x, y]`);
  }

  const x = finiteNumber(value[0], `${label}[0]`);
  const y = finiteNumber(value[1], `${label}[1]`);
  const maxX = Math.max(1, Math.round(frameSize.width) - 1);
  const maxY = Math.max(1, Math.round(frameSize.height) - 1);
  if (x < 0 || y < 0 || x > maxX + 1 || y > maxY + 1) {
    throw new Error(
      `${label} is outside the current frame (${frameSize.width}x${frameSize.height})`,
    );
  }

  return {
    source: [x, y],
    normalized: [
      Math.round((Math.min(x, maxX) * 1000) / maxX),
      Math.round((Math.min(y, maxY) * 1000) / maxY),
    ],
  };
};

const ensureNoModifier = (args: Record<string, unknown>, action: string) => {
  if (typeof args.text === "string" && args.text.trim()) {
    throw new Error(`computer.${action} modifiers are not supported`);
  }
};

const keyboardKeys = (value: unknown, action: string): string[] => {
  if (typeof value !== "string" || !value.trim()) {
    throw new Error(`computer.${action} requires a non-empty key`);
  }
  const keys = value.split("+").map((key) => key.trim()).filter(Boolean);
  if (keys.length < 1 || keys.length > 8 || keys.some((key) => [...key].length > 20)) {
    throw new Error(`computer.${action} supports 1..8 short key names`);
  }
  if (new Set(keys.map((key) => key.toLowerCase())).size !== keys.length) {
    throw new Error(`computer.${action} does not allow duplicate keys`);
  }
  return keys;
};

const adaptComputerCall = (
  call: ToolCall,
  context: AdapterContext,
): AdaptedCall => {
  let args: Record<string, unknown>;
  try {
    args = parseArguments(call);
  } catch (error) {
    return {
      call,
      event: rejectedEvent(call, null, error instanceof Error ? error.message : String(error)),
    };
  }

  const action = typeof args.action === "string" ? args.action : null;
  if (!action) {
    return {
      call,
      event: rejectedEvent(call, null, "computer.action is missing"),
    };
  }

  try {
    switch (action) {
      case "screenshot":
        return adaptedCall(call, action, "wait", {
          analysis: adapterAnalysis(action, "wait"),
          name: "Refresh current frame",
          seconds: 1,
        });
      case "wait": {
        const duration = finiteNumber(args.duration ?? 1, "computer.duration");
        return adaptedCall(call, action, "wait", {
          analysis: adapterAnalysis(action, "wait"),
          name: "Wait for current frame",
          seconds: Math.min(60, Math.max(1, Math.ceil(duration))),
        });
      }
      case "left_click":
      case "double_click": {
        ensureNoModifier(args, action);
        const point = normalizedCoordinate(args.coordinate, "computer.coordinate", context.frameSize);
        return adaptedCall(call, action, "click", {
          analysis: adapterAnalysis(action, "click"),
          name: action === "double_click" ? "Computer double click" : "Computer click",
          x: point.normalized[0],
          y: point.normalized[1],
          clicks: action === "double_click" ? 2 : 1,
        }, { source: [point.source], normalized: [point.normalized] });
      }
      case "mouse_move": {
        const point = normalizedCoordinate(args.coordinate, "computer.coordinate", context.frameSize);
        return adaptedCall(call, action, "hover", {
          analysis: adapterAnalysis(action, "hover"),
          name: "Computer mouse move",
          x: point.normalized[0],
          y: point.normalized[1],
        }, { source: [point.source], normalized: [point.normalized] });
      }
      case "left_click_drag": {
        ensureNoModifier(args, action);
        const start = normalizedCoordinate(
          args.start_coordinate,
          "computer.start_coordinate",
          context.frameSize,
        );
        const end = normalizedCoordinate(args.coordinate, "computer.coordinate", context.frameSize);
        return adaptedCall(call, action, "drag", {
          analysis: adapterAnalysis(action, "drag"),
          name: "Computer drag",
          fromX: start.normalized[0],
          fromY: start.normalized[1],
          toX: end.normalized[0],
          toY: end.normalized[1],
        }, {
          source: [start.source, end.source],
          normalized: [start.normalized, end.normalized],
        });
      }
      case "type": {
        if (typeof args.text !== "string" || !args.text) {
          throw new Error("computer.type requires non-empty text");
        }
        return adaptedCall(call, action, "keyboard_type", {
          analysis: adapterAnalysis(action, "keyboard_type"),
          name: "Computer keyboard text",
          text: args.text,
        });
      }
      case "key":
      case "keypress": {
        const rawKey = args.text ?? args.key;
        return adaptedCall(call, action, "keyboard_press", {
          analysis: adapterAnalysis(action, "keyboard_press"),
          name: "Computer keyboard key",
          keys: keyboardKeys(rawKey, action),
        });
      }
      default:
        return {
          call,
          event: rejectedEvent(call, action, `computer.${action} is not safely adaptable`),
        };
    }
  } catch (error) {
    return {
      call,
      event: rejectedEvent(call, action, error instanceof Error ? error.message : String(error)),
    };
  }
};

export function adaptUnexpectedComputerToolCalls(
  calls: ToolCall[],
  context: AdapterContext,
): { calls: ToolCall[]; events: ComputerToolAdapterEvent[] } {
  const events: ComputerToolAdapterEvent[] = [];
  const adaptedCalls = calls.map((call) => {
    if (call.function.name !== "computer" || context.offeredToolNames.has("computer")) {
      return call;
    }

    const adapted = adaptComputerCall(call, context);
    if (
      adapted.event.outcome === "adapted"
      && !context.offeredToolNames.has(adapted.call.function.name)
    ) {
      events.push(rejectedEvent(
        call,
        adapted.event.action,
        `mapped tool ${adapted.call.function.name} was not offered this turn`,
      ));
      return call;
    }

    events.push(adapted.event);
    return adapted.call;
  });

  return { calls: adaptedCalls, events };
}
