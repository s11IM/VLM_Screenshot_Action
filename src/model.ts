import type {
  Attachment,
  ChatCompletionMessage,
  Conversation,
  Message,
  Settings,
  ToolCall,
} from "./types";

export const SYSTEM_PROMPT = `你通过截图操作游戏，只能在用户框选的区域内点击、拖拽、悬停，或向当前前台窗口输入键盘，或用 wait 等待、end_round 结束本次运行。

坐标只来自标记为 CURRENT_FRAME 的最新画面：把它当作 1000x1000 的相对平面（左上 0,0，中心 500,500，右下 1000,1000），按目标所在的相对位置直接输出坐标。画面中的红色十字标记是你上一次操作的落点，可据此判断偏差。不要用更早的画面或用户上传的参考图定位。

键盘操作目标必须与 CURRENT_FRAME 截图时的前台窗口一致，窗口变化会安全中断本轮。keyboard_type 用于输入字面 Unicode 文本（可包含中文、换行和 Tab），keyboard_press 用于单键、组合键和短时间保持的游戏按键。输入框或游戏窗口需要先用 click 获取焦点；每次操作后都要依据新截图确认效果。F8 保留为用户键盘输入急停键，不可调用。

还有操作就调用一个工具；不调用工具的回复会被当作本次运行结束。`;

const STRATEGY_ANALYSIS_PROPERTY = {
  type: "string",
  maxLength: 1000,
  description: "简述本步依据当前画面做什么、为什么。",
} as const;

export const ACTION_TOOLS = [
  {
    type: "function",
    function: {
      name: "click",
      description: "在 CURRENT_FRAME 的相对位置执行左键单击或双击。直接使用唯一的 NORMALIZED_0_1000 坐标，不要计算原图像素。",
      parameters: {
        type: "object",
        properties: {
          analysis: STRATEGY_ANALYSIS_PROPERTY,
          name: {
            type: "string",
            minLength: 1,
            maxLength: 120,
            description: "本次点击的简短名称。",
          },
          x: {
            type: "integer",
            minimum: 0,
            maximum: 1000,
            description: "目标在 CURRENT_FRAME 中的相对水平位置：左=0，中心=500，右=1000。",
          },
          y: {
            type: "integer",
            minimum: 0,
            maximum: 1000,
            description: "目标在 CURRENT_FRAME 中的相对垂直位置：上=0，中心=500，下=1000。",
          },
          clicks: {
            type: "integer",
            enum: [1, 2],
            description: "1 为单击，2 为双击。",
          },
        },
        required: ["analysis", "name", "x", "y", "clicks"],
        additionalProperties: false,
      },
    },
  },
  {
    type: "function",
    function: {
      name: "drag",
      description: "在 CURRENT_FRAME 中按相对位置拖拽。起点和终点都直接使用唯一的 NORMALIZED_0_1000 坐标，不要计算原图像素。",
      parameters: {
        type: "object",
        properties: {
          analysis: STRATEGY_ANALYSIS_PROPERTY,
          name: {
            type: "string",
            minLength: 1,
            maxLength: 120,
            description: "本次拖拽的简短名称。",
          },
          fromX: { type: "integer", minimum: 0, maximum: 1000 },
          fromY: { type: "integer", minimum: 0, maximum: 1000 },
          toX: { type: "integer", minimum: 0, maximum: 1000 },
          toY: { type: "integer", minimum: 0, maximum: 1000 },
        },
        required: ["analysis", "name", "fromX", "fromY", "toX", "toY"],
        additionalProperties: false,
      },
    },
  },
  {
    type: "function",
    function: {
      name: "hover",
      description: "把鼠标移动到 CURRENT_FRAME 的相对位置但不点击。直接使用唯一的 NORMALIZED_0_1000 坐标，不要计算原图像素。",
      parameters: {
        type: "object",
        properties: {
          analysis: STRATEGY_ANALYSIS_PROPERTY,
          name: {
            type: "string",
            minLength: 1,
            maxLength: 120,
            description: "本次悬停目标的简短名称。",
          },
          x: {
            type: "integer",
            minimum: 0,
            maximum: 1000,
            description: "目标在 CURRENT_FRAME 中的相对水平位置：左=0，中心=500，右=1000。",
          },
          y: {
            type: "integer",
            minimum: 0,
            maximum: 1000,
            description: "目标在 CURRENT_FRAME 中的相对垂直位置：上=0，中心=500，下=1000。",
          },
        },
        required: ["analysis", "name", "x", "y"],
        additionalProperties: false,
      },
    },
  },
  {
    type: "function",
    function: {
      name: "keyboard_type",
      description: "向当前前台窗口输入字面 Unicode 文本，可包含中文。先用 click 让输入框或游戏窗口获得焦点；换行会按 Enter，Tab 会按 Tab，不使用剪贴板。每次调用后根据新截图确认结果。",
      parameters: {
        type: "object",
        properties: {
          analysis: STRATEGY_ANALYSIS_PROPERTY,
          name: {
            type: "string",
            minLength: 1,
            maxLength: 120,
            description: "本次键盘输入的简短名称。",
          },
          text: {
            type: "string",
            minLength: 1,
            maxLength: 10000,
            description: "要输入的字面文本，不要把快捷键写在这里。",
          },
          intervalMs: {
            type: "integer",
            minimum: 0,
            maximum: 1000,
            description: "每个字符之间的延迟毫秒数，默认 0；总输入延迟最多 30000 毫秒。",
          },
          submit: {
            type: "boolean",
            description: "输入完成后是否额外按一次 Enter，默认 false。",
          },
        },
        required: ["analysis", "name", "text"],
        additionalProperties: false,
      },
    },
  },
  {
    type: "function",
    function: {
      name: "keyboard_press",
      description: "向截图时的前台窗口按下一个物理按键或组合键，并在短暂保持后释放。用于 Enter、方向键、Ctrl+A、Shift 或游戏 WASD 等；不用于输入文本。F8 保留为用户急停键，不可调用。每次调用后根据新截图确认结果。",
      parameters: {
        type: "object",
        properties: {
          analysis: STRATEGY_ANALYSIS_PROPERTY,
          name: {
            type: "string",
            minLength: 1,
            maxLength: 120,
            description: "本次按键的简短名称。",
          },
          keys: {
            type: "array",
            minItems: 1,
            maxItems: 8,
            uniqueItems: true,
            items: {
              type: "string",
              minLength: 1,
              maxLength: 20,
              description: "按键名称，如 CTRL、A、ENTER、F1、ARROWLEFT。",
            },
            description: "同时按下的按键，按数组顺序按下并逆序释放。",
          },
          holdMs: {
            type: "integer",
            minimum: 0,
            maximum: 10000,
            description: "保持按下的毫秒数，默认 50；游戏移动可使用更长时间。",
          },
        },
        required: ["analysis", "name", "keys"],
        additionalProperties: false,
      },
    },
  },
  {
    type: "function",
    function: {
      name: "wait",
      description: "不执行任何鼠标操作，等待指定秒数后重新截图并继续运行。用于对手回合、动画播放或画面加载。",
      parameters: {
        type: "object",
        properties: {
          analysis: STRATEGY_ANALYSIS_PROPERTY,
          name: {
            type: "string",
            minLength: 1,
            maxLength: 120,
            description: "等待原因的简短名称。",
          },
          seconds: {
            type: "integer",
            minimum: 1,
            maximum: 60,
            description: "等待秒数。",
          },
        },
        required: ["analysis", "name", "seconds"],
        additionalProperties: false,
      },
    },
  },
  {
    type: "function",
    function: {
      name: "end_round",
      description: "结束本次运行。仅在目标完成、需要用户决策或长时间无需操作时调用。",
      parameters: {
        type: "object",
        properties: {
          analysis: STRATEGY_ANALYSIS_PROPERTY,
          message: {
            type: "string",
            minLength: 1,
            maxLength: 500,
            description: "给用户看的本次运行总结。",
          },
        },
        required: ["analysis", "message"],
        additionalProperties: false,
      },
    },
  },
] as const;

export function availableActionTools(toolsEnabled: boolean, actionableFrameAvailable = true) {
  if (!toolsEnabled || !actionableFrameAvailable) return [];
  return ACTION_TOOLS;
}

export function latestRoundFrame(conversation: Conversation, roundId: string) {
  return conversation.messages
    .filter((message) => message.roundId === roundId)
    .flatMap((message) => message.attachments ?? [])
    .filter((attachment) => attachment.source !== "upload")
    .at(-1);
}

type ImageFrame = "PREVIOUS_FRAME" | "REFERENCE_IMAGE";

export function imageStateText(attachment: Attachment, frame?: ImageFrame) {
  const imageFrame = frame ?? (attachment.source === "upload" ? "REFERENCE_IMAGE" : "PREVIOUS_FRAME");
  return `[IMAGE frame=${imageFrame} coordinates=NORMALIZED_0_1000]`;
}

export function imageTimeText(attachment: Attachment) {
  const capturedAt = attachment.capturedAt;
  const date = new Date(capturedAt ?? Date.now());
  const validDate = typeof capturedAt === "number"
    && Number.isFinite(capturedAt)
    && !Number.isNaN(date.getTime())
    ? date
    : new Date();
  const part = (value: number) => String(value).padStart(2, "0");
  const timestamp = `${part(validDate.getHours())}:${part(validDate.getMinutes())}:${part(validDate.getSeconds())}`;
  return attachment.source === "upload"
    ? `[REFERENCE_IMAGE_TIME ${timestamp}]`
    : `[SCREENSHOT_TIME ${timestamp}]`;
}

function messageContent(message: Message): ChatCompletionMessage["content"] {
  const images = message.attachments ?? [];
  if (images.length === 0) return message.text;
  const orderedImages = [
    ...images.filter((image) => image.source === "upload"),
    ...images.filter((image) => image.source !== "upload"),
  ];

  return [
    ...(message.text.trim() ? [{ type: "text", text: message.text }] : []),
    ...orderedImages.flatMap((image) => [
      { type: "text", text: imageTimeText(image) },
      { type: "text", text: imageStateText(image) },
      { type: "image_url", image_url: { url: image.dataUrl, detail: "high" } },
    ]),
  ];
}

export function buildMessages(
  conversation: Conversation,
  settings: Settings,
  systemPrompt: string,
): ChatCompletionMessage[] {
  const cycleIds: string[] = [];
  const grouped = conversation.messages.map((message, index) => {
    const cycleId = message.cycleId ?? "";
    if (message.role !== "system" && cycleId && !cycleIds.includes(cycleId)) {
      cycleIds.push(cycleId);
    }
    return { message, cycleId, index };
  });

  const selectedCycleIds = new Set(cycleIds.slice(-settings.contextCycles));
  const relevant = grouped.filter(({ cycleId }) => selectedCycleIds.has(cycleId));
  const activeTask = grouped.slice().reverse().find(({ message }) =>
    message.role === "user" && Boolean(message.text.trim()),
  );
  const lastRoundSummary = grouped.slice().reverse().find(({ message, index }) =>
    message.role === "assistant"
    && message.kind === "final"
    && Boolean(message.text.trim())
    && index > (activeTask?.index ?? -1),
  );
  const messages: ChatCompletionMessage[] = [{
    role: "system",
    content: systemPrompt.trim()
      ? `${SYSTEM_PROMPT}\n\n## 用户补充指令\n${systemPrompt.trim()}`
      : SYSTEM_PROMPT,
  }];

  if (activeTask && !selectedCycleIds.has(activeTask.cycleId)) {
    messages.push({
      role: "user",
      content: `[ACTIVE_TASK]\n${activeTask.message.text.trim()}`,
    });
  }
  if (lastRoundSummary && !selectedCycleIds.has(lastRoundSummary.cycleId)) {
    messages.push({
      role: "assistant",
      content: `[LAST_ROUND_SUMMARY]\n${lastRoundSummary.message.text.trim()}`,
    });
  }

  for (const { message, index } of relevant) {
    if (message.role === "user") {
      messages.push({ role: "user", content: messageContent(message) });
    } else if (
      message.role === "assistant"
      && message.kind === "strategy"
      && message.text.trim()
    ) {
      messages.push({
        role: "assistant",
        content: `[STRATEGY_ANALYSIS]\n${message.text.trim()}`,
      });
    } else if (message.role === "tool" && message.contextText?.trim()) {
      messages.push({ role: "user", content: message.contextText.trim() });
    } else if (
      message.role === "assistant"
      && message.kind === "final"
      && message.text.trim()
      && index === lastRoundSummary?.index
    ) {
      messages.push({
        role: "assistant",
        content: `[LAST_ROUND_SUMMARY]\n${message.text.trim()}`,
      });
    }
  }

  return messages;
}

export function expireOldImages(
  messages: ChatCompletionMessage[],
  retainedImageCount = 1,
): ChatCompletionMessage[] {
  const images: Array<{
    imageKey: string;
    labelKey: string | null;
    reference: boolean;
  }> = [];
  messages.forEach((message, messageIndex) => {
    if (!Array.isArray(message.content)) return;
    let labelKey: string | null = null;
    let reference = false;
    message.content.forEach((part, partIndex) => {
      if (part.type === "text" && typeof part.text === "string" && part.text.startsWith("[IMAGE frame=")) {
        labelKey = `${messageIndex}:${partIndex}`;
        reference = part.text.startsWith("[IMAGE frame=REFERENCE_IMAGE ");
      }
      if (part.type === "image_url") {
        images.push({ imageKey: `${messageIndex}:${partIndex}`, labelKey, reference });
        labelKey = null;
        reference = false;
      }
    });
  });
  const retainedStart = Math.max(0, images.length - Math.max(0, retainedImageCount));
  const expired = images.slice(0, retainedStart);
  const retainedFrames = images.slice(retainedStart).filter((image) => !image.reference);
  const currentFrame = retainedFrames.at(-1);
  const expiredImageKeys = new Set(expired.map((image) => image.imageKey));
  const expiredLabelKeys = new Set(expired.flatMap((image) => image.labelKey ? [image.labelKey] : []));
  const currentLabelKey = currentFrame?.labelKey ?? null;

  return messages.map((message, messageIndex) => {
    if (!Array.isArray(message.content)) return message;
    return {
      ...message,
      content: message.content.flatMap((part, partIndex) => {
        const key = `${messageIndex}:${partIndex}`;
        if (expiredLabelKeys.has(key)) return [{ type: "text", text: "[IMAGE frame=OMITTED]" }];
        if (expiredImageKeys.has(key)) return [];
        if (
          part.type === "text"
          && typeof part.text === "string"
          && key === currentLabelKey
        ) {
          return [{
            ...part,
            text: part.text.replace(/^\[IMAGE frame=[A-Z_]+/, "[IMAGE frame=CURRENT_FRAME"),
          }];
        }
        return [part];
      }),
    };
  });
}

export function parseToolArguments(call: ToolCall): Record<string, unknown> {
  let parsed: unknown;
  try {
    parsed = JSON.parse(call.function.arguments);
  } catch {
    throw new Error(`模型返回了无效的 ${call.function.name} 参数`);
  }
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error(`${call.function.name} 参数必须是 JSON 对象`);
  }
  const args = parsed as Record<string, unknown>;
  const normalizedNumber = (
    name: string,
    raw: unknown,
    minimum: number,
    maximum: number,
    options: { allowed?: number[]; round?: boolean } = {},
  ) => {
    const scalar = Array.isArray(raw) && raw.length === 1 ? raw[0] : raw;
    const value = typeof scalar === "string" && /^-?\d+(?:\.0+)?$/.test(scalar.trim())
      ? Number(scalar)
      : scalar;
    if (typeof value !== "number" || !Number.isFinite(value)) {
      throw new Error(`${call.function.name}.${name} 必须是数字`);
    }
    const normalized = options.round ? Math.round(value) : value;
    if (!Number.isInteger(normalized) || normalized < minimum || normalized > maximum) {
      throw new Error(`${call.function.name}.${name} 必须是 ${minimum}..${maximum} 范围内的整数`);
    }
    if (options.allowed && !options.allowed.includes(normalized)) {
      throw new Error(`${call.function.name}.${name} 不支持值 ${normalized}`);
    }
    return normalized;
  };
  const numberValue = (
    name: string,
    minimum: number,
    maximum: number,
    options: { defaultValue?: number; allowed?: number[]; round?: boolean } = {},
  ) => normalizedNumber(name, args[name] ?? options.defaultValue, minimum, maximum, options);
  const stringValue = (name: string, options: { defaultValue?: string; maxLength?: number } = {}) => {
    const value = args[name] ?? options.defaultValue;
    if (typeof value !== "string" || !value.trim()) {
      throw new Error(`${call.function.name}.${name} 必须是非空字符串`);
    }
    if (options.maxLength && [...value].length > options.maxLength) {
      throw new Error(`${call.function.name}.${name} 不能超过 ${options.maxLength} 个字符`);
    }
    return value;
  };
  const textValue = (name: string, maxLength: number) => {
    const value = args[name];
    if (typeof value !== "string" || value.length === 0) {
      throw new Error(`${call.function.name}.${name} 必须是字符串`);
    }
    if ([...value].length > maxLength) {
      throw new Error(`${call.function.name}.${name} 不能超过 ${maxLength} 个字符`);
    }
    return value;
  };
  const booleanValue = (name: string, defaultValue: boolean) => {
    const value = args[name] ?? defaultValue;
    if (typeof value !== "boolean") {
      throw new Error(`${call.function.name}.${name} 必须是布尔值`);
    }
    return value;
  };
  const keysValue = (name: string) => {
    const value = args[name];
    if (!Array.isArray(value) || value.length < 1 || value.length > 8) {
      throw new Error(`${call.function.name}.${name} 必须包含 1..8 个按键`);
    }
    const keys = value.map((key, index) => {
      if (typeof key !== "string" || !key.trim() || [...key].length > 20) {
        throw new Error(`${call.function.name}.${name}[${index}] 必须是 1..20 个字符的按键名称`);
      }
      return key;
    });
    if (new Set(keys.map((key) => key.toLowerCase())).size !== keys.length) {
      throw new Error(`${call.function.name}.${name} 不允许重复按键`);
    }
    return keys;
  };
  switch (call.function.name) {
    case "click":
      return {
        analysis: stringValue("analysis", {
          defaultValue: "已观察当前截图并选择该操作，下一张截图需要确认界面反馈。",
          maxLength: 1000,
        }),
        name: stringValue("name", { maxLength: 120 }),
        x: numberValue("x", 0, 1000, { round: true }),
        y: numberValue("y", 0, 1000, { round: true }),
        clicks: numberValue("clicks", 1, 2, { allowed: [1, 2] }),
      };
    case "drag":
      return {
        analysis: stringValue("analysis", {
          defaultValue: "已观察当前截图并选择该操作，下一张截图需要确认界面反馈。",
          maxLength: 1000,
        }),
        name: stringValue("name", { maxLength: 120 }),
        fromX: numberValue("fromX", 0, 1000, { round: true }),
        fromY: numberValue("fromY", 0, 1000, { round: true }),
        toX: numberValue("toX", 0, 1000, { round: true }),
        toY: numberValue("toY", 0, 1000, { round: true }),
      };
    case "hover":
      return {
        analysis: stringValue("analysis", {
          defaultValue: "已观察当前截图并选择该操作，下一张截图需要确认界面反馈。",
          maxLength: 1000,
        }),
        name: stringValue("name", { maxLength: 120 }),
        x: numberValue("x", 0, 1000, { round: true }),
        y: numberValue("y", 0, 1000, { round: true }),
      };
    case "keyboard_type":
      return {
        analysis: stringValue("analysis", {
          defaultValue: "已观察当前截图并选择该输入，下一张截图需要确认界面反馈。",
          maxLength: 1000,
        }),
        name: stringValue("name", { maxLength: 120 }),
        text: textValue("text", 10000),
        intervalMs: numberValue("intervalMs", 0, 1000, { defaultValue: 0 }),
        submit: booleanValue("submit", false),
      };
    case "keyboard_press":
      return {
        analysis: stringValue("analysis", {
          defaultValue: "已观察当前截图并选择该按键，下一张截图需要确认界面反馈。",
          maxLength: 1000,
        }),
        name: stringValue("name", { maxLength: 120 }),
        keys: keysValue("keys"),
        holdMs: numberValue("holdMs", 0, 10000, { defaultValue: 50 }),
      };
    case "wait":
      return {
        analysis: stringValue("analysis", {
          defaultValue: "当前画面仍在变化或需要等待，下一张截图需要确认新状态。",
          maxLength: 1000,
        }),
        name: stringValue("name", { maxLength: 120 }),
        seconds: numberValue("seconds", 1, 60, { round: true }),
      };
    case "end_round":
      return {
        analysis: stringValue("analysis", {
          defaultValue: "已根据当前截图判断本次运行可以结束。",
          maxLength: 1000,
        }),
        message: stringValue("message", { maxLength: 500 }),
      };
    default:
      throw new Error(`模型请求了不支持的工具 ${call.function.name}`);
  }
}
