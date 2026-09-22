import {
  Bot,
  Camera,
  ChevronRight,
  CircleStop,
  Crop,
  Eye,
  EyeOff,
  Gamepad2,
  History,
  ImagePlus,
  LoaderCircle,
  Maximize2,
  Minimize2,
  MousePointer2,
  Pause,
  Plus,
  Send,
  Settings as SettingsIcon,
  Sparkles,
  Trash2,
  Upload,
  X,
} from "lucide-react";
import {
  useEffect,
  useRef,
  useState,
  type ChangeEvent,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import {
  availableActionTools,
  buildMessages,
  expireOldImages,
  latestRoundFrame,
  parseToolArguments,
} from "./model";
import { adaptUnexpectedComputerToolCalls } from "./computerToolAdapter";
import { loadStoredConversations, storeConversations } from "./storage";
import {
  beginOperation,
  cancelOperation,
  captureRegion,
  enterMiniMode,
  executeInputAction,
  exitMiniMode,
  finishOperation,
  isTauri,
  logClientEvent,
  requestChatCompletion,
  selectRegion,
} from "./tauri";
import type {
  Attachment,
  ChatCompletionMessage,
  Conversation,
  Message,
  Region,
  Settings,
  ToolCall,
} from "./types";

const DEFAULT_SETTINGS: Settings = {
  apiUrl: "https://api.openai.com/v1",
  apiKey: "",
  model: "gpt-4o-mini",
  reasoningEffort: "low",
  contextCycles: 3,
  replyCaptureDelay: 15,
  operationCaptureDelay: 7,
  requestTimeout: 60,
  retryDelay: 3,
  retryOnFailure: true,
  toolsEnabled: true,
  autoCaptureAfterTool: true,
};

const DEFAULT_SYSTEM_PROMPT = "优先完成目标，并遵守游戏界面中可见的规则(如有)。";

const uid = () => crypto.randomUUID();
const createAttachment = (
  dataUrl: string,
  source: Attachment["source"],
  dimensions?: Pick<Region, "width" | "height">,
  frameId?: string,
): Attachment => {
  const id = uid();
  const capturedAt = Date.now();
  return {
    id,
    dataUrl,
    source,
    width: dimensions?.width,
    height: dimensions?.height,
    capturedAt,
    frameId,
  };
};
const HISTORY_RETENTION_DAYS = 15;
const HISTORY_RETENTION_MS = HISTORY_RETENTION_DAYS * 24 * 60 * 60 * 1000;

const newConversation = (number: number): Conversation => ({
  id: uid(),
  title: `新建会话 #${number}`,
  createdAt: Date.now(),
  updatedAt: Date.now(),
  messages: [],
});

const pruneExpiredConversations = (
  conversations: Conversation[],
  now = Date.now(),
) => {
  const cutoff = now - HISTORY_RETENTION_MS;
  const kept = conversations.filter((conversation) => {
    const timestamp = Number.isFinite(conversation.updatedAt)
      ? conversation.updatedAt
      : conversation.createdAt;
    return !Number.isFinite(timestamp) || timestamp >= cutoff;
  });
  return { kept, removedCount: conversations.length - kept.length };
};

const loadJson = <T,>(key: string, fallback: T): T => {
  try {
    const raw = localStorage.getItem(key);
    return raw ? (JSON.parse(raw) as T) : fallback;
  } catch {
    return fallback;
  }
};

const clampNumber = (value: unknown, min: number, max: number, fallback: number) =>
  Number.isFinite(value) ? Math.min(max, Math.max(min, value as number)) : fallback;

const normalizeSettings = (value: Partial<Settings>): Settings => ({
  ...DEFAULT_SETTINGS,
  ...value,
  contextCycles: clampNumber(value.contextCycles, 1, 999, DEFAULT_SETTINGS.contextCycles),
  replyCaptureDelay: clampNumber(value.replyCaptureDelay, 5, 120, DEFAULT_SETTINGS.replyCaptureDelay),
  operationCaptureDelay: clampNumber(value.operationCaptureDelay, 0.5, 20, DEFAULT_SETTINGS.operationCaptureDelay),
  requestTimeout: clampNumber(value.requestTimeout, 5, 300, DEFAULT_SETTINGS.requestTimeout),
  retryDelay: clampNumber(value.retryDelay, 0, 15, DEFAULT_SETTINGS.retryDelay),
  reasoningEffort: value.reasoningEffort === "xhigh"
    ? "xhigh"
    : value.reasoningEffort === "high"
      ? "high"
      : "low",
  toolsEnabled: value.toolsEnabled ?? DEFAULT_SETTINGS.toolsEnabled,
  autoCaptureAfterTool:
    value.autoCaptureAfterTool ?? DEFAULT_SETTINGS.autoCaptureAfterTool,
});

const isCancelledError = (error: unknown) => /取消|中断/.test(String(error));
const toolResultJson = (payload: Record<string, unknown>) => JSON.stringify(payload);

const extractAssistantText = (content: ChatCompletionMessage["content"]): string => {
  if (typeof content === "string") return content;
  if (Array.isArray(content)) {
    return content
      .map((part) => {
        if (part && typeof part === "object" && "type" in part) {
          const record = part as Record<string, unknown>;
          if (record.type === "text" && typeof record.text === "string") return record.text;
        }
        return "";
      })
      .join("")
      .trim();
  }
  return "";
};

const ensureToolCallId = (call: ToolCall): ToolCall =>
  call.id && call.id.trim() ? call : { ...call, id: `tc-${uid()}` };

const ACTION_RESULT_REASONS: Record<string, string> = {
  TOOLS_DISABLED: "应用设置已关闭模型工具",
  NO_CURRENT_FRAME: "没有可用于定位的当前截图",
  INVALID_TOOL_ARGUMENTS: "模型返回的工具参数无效",
  EXECUTION_FAILED: "系统未能执行该动作",
};

const ACTION_RESULT_FIXES: Record<string, string> = {
  TOOLS_DISABLED: "本次运行未向模型提供工具，请直接回复用户。",
  NO_CURRENT_FRAME: "请等待最新截图出现后再定位并调用工具。",
  INVALID_TOOL_ARGUMENTS: "请按该工具要求补全全部必填参数，坐标用 0-1000，再重新调用。",
  EXECUTION_FAILED: "请依据最新画面重新判断目标位置后再试一次。",
};

const actionResultContextText = (
  tool: string,
  status: "success" | "failed",
  code?: string,
  detail?: string,
) => {
  const safeTool = tool.replace(/[^a-zA-Z0-9_-]/g, "").slice(0, 40) || "unknown";
  if (status === "success") {
    return `[ACTION_RESULT tool=${safeTool} status=SUCCESS]`;
  }
  const safeCode = (code ?? "EXECUTION_FAILED")
    .replace(/[^A-Z0-9_-]/gi, "")
    .slice(0, 60)
    .toUpperCase() || "EXECUTION_FAILED";
  const reason = (detail?.trim() || ACTION_RESULT_REASONS[safeCode] || "动作调用失败")
    .replace(/[\r\n]+/g, " ")
    .slice(0, 180);
  const fix = (ACTION_RESULT_FIXES[safeCode] || "请依据最新画面重新判断后再试。")
    .replace(/[\r\n]+/g, " ")
    .slice(0, 180);
  return `[ACTION_RESULT tool=${safeTool} status=FAILED code=${safeCode} reason=${reason} fix=${fix}]`;
};

const sleepWithCancel = async (ms: number, isCancelled: () => boolean) => {
  const step = 250;
  let elapsed = 0;
  while (elapsed < ms) {
    await new Promise((resolve) => window.setTimeout(resolve, Math.min(step, ms - elapsed)));
    elapsed += step;
    if (isCancelled()) return false;
  }
  return !isCancelled();
};

function App() {
  const [settings, setSettings] = useState(() =>
    normalizeSettings(loadJson("vlm_screenshot_action.settings", DEFAULT_SETTINGS)),
  );
  const [systemPrompt, setSystemPrompt] = useState(() =>
    localStorage.getItem("vlm_screenshot_action.systemPrompt") ?? DEFAULT_SYSTEM_PROMPT,
  );
  const [conversations, setConversations] = useState<Conversation[]>(() => [newConversation(1)]);
  const [activeId, setActiveId] = useState(() =>
    localStorage.getItem("vlm_screenshot_action.activeId") ?? conversations[0].id,
  );
  const [region, setRegion] = useState<Region | null>(() =>
    loadJson<Region | null>("vlm_screenshot_action.region", null),
  );
  const [page, setPage] = useState<"chat" | "settings">("chat");
  const [historyOpen, setHistoryOpen] = useState(false);
  const [storageReady, setStorageReady] = useState(false);
  const [draft, setDraft] = useState("");
  const [pendingAttachments, setPendingAttachments] = useState<Attachment[]>([]);
  const [isBusy, setIsBusy] = useState(false);
  const [isCancelling, setIsCancelling] = useState(false);
  const [replyCaptureEnabled, setReplyCaptureEnabled] = useState(false);
  const [miniMode, setMiniMode] = useState(false);
  const [showApiKey, setShowApiKey] = useState(false);
  const [notice, setNotice] = useState("请选择目标区域，然后发送第一条消息");
  const fileInputRef = useRef<HTMLInputElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const busyRef = useRef(false);
  const operationRef = useRef<string | null>(null);
  const roundRef = useRef<string | null>(null);
  const replyCaptureTimerRef = useRef<number | null>(null);
  const replyCaptureEnabledRef = useRef(false);

  const activeConversation =
    conversations.find((item) => item.id === activeId) ?? conversations[0];
  const conversationsRef = useRef(conversations);
  const activeIdRef = useRef(activeId);
  const regionRef = useRef(region);
  conversationsRef.current = conversations;
  activeIdRef.current = activeId;
  regionRef.current = region;

  useEffect(() => {
    localStorage.removeItem("vlm_screenshot_action.modelMemory");
    logClientEvent("ui.ready", {
      conversationCount: conversations.length,
      hasSavedRegion: Boolean(region),
    });
    const onError = (event: globalThis.ErrorEvent) => {
      logClientEvent(
        "ui.unhandled_error",
        { message: event.message, source: event.filename, line: event.lineno },
        "error",
      );
    };
    const onRejection = (event: PromiseRejectionEvent) => {
      logClientEvent(
        "ui.unhandled_rejection",
        { message: String(event.reason) },
        "error",
      );
    };
    window.addEventListener("error", onError);
    window.addEventListener("unhandledrejection", onRejection);
    return () => {
      window.removeEventListener("error", onError);
      window.removeEventListener("unhandledrejection", onRejection);
    };
  }, []);

  useEffect(() => {
    let active = true;
    void loadStoredConversations()
      .then((stored) => {
        if (!active || stored.length === 0) return;
        const { kept, removedCount } = pruneExpiredConversations(stored);
        const next = kept.length > 0 ? kept : [newConversation(1)];
        if (removedCount > 0) {
          logClientEvent("history.expired_pruned", {
            removedCount,
            retentionDays: HISTORY_RETENTION_DAYS,
            source: "indexedDB",
          });
        }
        setConversations(next);
        setActiveId((current) =>
          next.some((conversation) => conversation.id === current)
            ? current
            : next[0].id,
        );
      })
      .catch((error) =>
        logClientEvent("storage.indexeddb_load_failed", { error: String(error) }, "warn"),
      )
      .finally(() => {
        if (active) setStorageReady(true);
      });
    return () => {
      active = false;
    };
  }, []);

  useEffect(() => {
    const prune = () => {
      setConversations((current) => {
        const protectedConversationId = busyRef.current ? activeId : null;
        const cutoff = Date.now() - HISTORY_RETENTION_MS;
        const kept = current.filter((conversation) => {
          if (conversation.id === protectedConversationId) return true;
          const timestamp = Number.isFinite(conversation.updatedAt)
            ? conversation.updatedAt
            : conversation.createdAt;
          return !Number.isFinite(timestamp) || timestamp >= cutoff;
        });
        const removedCount = current.length - kept.length;
        if (removedCount === 0) return current;
        logClientEvent("history.expired_pruned", {
          removedCount,
          retentionDays: HISTORY_RETENTION_DAYS,
          source: "runtime",
        });
        return kept.length > 0 ? kept : [newConversation(1)];
      });
    };
    const timer = window.setInterval(prune, 60 * 60 * 1000);
    return () => window.clearInterval(timer);
  }, [activeId]);

  useEffect(() => {
    if (!conversations.some((conversation) => conversation.id === activeId)) {
      setActiveId(conversations[0].id);
    }
  }, [activeId, conversations]);

  useEffect(() => {
    localStorage.setItem("vlm_screenshot_action.settings", JSON.stringify(settings));
  }, [settings]);

  useEffect(() => {
    localStorage.setItem("vlm_screenshot_action.systemPrompt", systemPrompt);
  }, [systemPrompt]);

  useEffect(() => {
    if (!storageReady) return;
    const timer = window.setTimeout(() => {
      void storeConversations(conversations).catch((error) =>
        logClientEvent("storage.indexeddb_write_failed", { error: String(error) }, "warn"),
      );
    }, 180);
    return () => window.clearTimeout(timer);
  }, [conversations, storageReady]);

  useEffect(() => {
    localStorage.setItem("vlm_screenshot_action.activeId", activeId);
  }, [activeId]);

  useEffect(() => {
    localStorage.setItem("vlm_screenshot_action.region", JSON.stringify(region));
  }, [region]);

  useEffect(() => {
    replyCaptureEnabledRef.current = replyCaptureEnabled;
    if (!replyCaptureEnabled && replyCaptureTimerRef.current !== null) {
      window.clearTimeout(replyCaptureTimerRef.current);
      replyCaptureTimerRef.current = null;
      logClientEvent("reply_capture.cancelled", { reason: "disabled" });
    }
  }, [replyCaptureEnabled]);

  useEffect(() => () => {
    if (replyCaptureTimerRef.current !== null) {
      window.clearTimeout(replyCaptureTimerRef.current);
    }
  }, []);

  useEffect(() => {
    scrollRef.current?.scrollTo({
      top: scrollRef.current.scrollHeight,
      behavior: "smooth",
    });
  }, [activeConversation.messages, isBusy]);

  const updateConversation = (
    id: string,
    updater: (conversation: Conversation) => Conversation,
  ) => {
    setConversations((current) =>
      current.map((conversation) =>
        conversation.id === id ? updater(conversation) : conversation,
      ),
    );
  };

  const appendMessage = (conversationId: string, message: Message) => {
    updateConversation(conversationId, (conversation) => ({
      ...conversation,
      updatedAt: Date.now(),
      messages: [...conversation.messages, message],
    }));
  };

  const takeScreenshot = async (source: Attachment["source"] = "manual") => {
    if (!region) {
      setNotice("请先选择截图区域");
      return null;
    }
    if (!isTauri()) {
      setNotice("浏览器预览不支持系统截图，请运行 Tauri 桌面端");
      return null;
    }

    logClientEvent("capture.requested", { source, region });
    setNotice("正在截取目标区域...");
    try {
      const capture = await captureRegion(region, { captureKind: source });
      const attachment = createAttachment(capture.dataUrl, source, region, capture.frameId);
      setPendingAttachments((current) => [...current, attachment]);
      setNotice("截图已加入输入框");
      return attachment;
    } catch (error) {
      setNotice(`截图失败：${String(error)}`);
      logClientEvent("capture.request_failed", { source, error: String(error) }, "error");
      throw error;
    }
  };

  const chooseRegion = async () => {
    if (!isTauri()) {
      setRegion({ x: 80, y: 80, width: 1280, height: 720 });
      setNotice("已设置浏览器演示区域；桌面端可实际框选屏幕");
      return;
    }
    setNotice("在遮罩上拖动鼠标框选目标区域，Esc 取消");
    try {
      const selected = await selectRegion();
      setRegion(selected);
      setNotice("目标区域已更新");
    } catch (error) {
      setNotice(String(error));
    }
  };

  const buildBody = (
    messages: ChatCompletionMessage[],
    tools: readonly unknown[] = [],
  ) => {
    const body: Record<string, unknown> = {
      model: settings.model,
      messages,
    };
    if (tools.length > 0) {
      body.tools = tools;
      body.tool_choice = "auto";
      body.parallel_tool_calls = false;
    }
    body.reasoning_effort = settings.reasoningEffort;
    return body;
  };

  const requestWithRetry = async (
    messages: ChatCompletionMessage[],
    operationId: string,
    roundId: string,
    requestIndex: number,
    tools: readonly unknown[],
  ) => {
    const attempts = settings.retryOnFailure ? 2 : 1;
    const preparedMessages = expireOldImages(messages, 1);
    let lastError: unknown;
    for (let attempt = 0; attempt < attempts; attempt += 1) {
      try {
        return await requestChatCompletion({
          apiUrl: settings.apiUrl,
          apiKey: settings.apiKey,
          body: buildBody(preparedMessages, tools),
          operationId,
          roundId,
          requestIndex,
          attempt: attempt + 1,
          toolsAllowed: tools.length > 0,
          timeoutSeconds: settings.requestTimeout,
        });
      } catch (initialError) {
        lastError = initialError;
        if (isCancelledError(initialError)) throw initialError;
        if (attempt + 1 < attempts) {
          logClientEvent("api.retry_scheduled", {
            operationId,
            roundId,
            requestIndex,
            failedAttempt: attempt + 1,
            retryDelaySeconds: settings.retryDelay,
            error: String(initialError),
          }, "warn");
          setNotice(`请求失败或超时，${settings.retryDelay} 秒后重试一次...`);
          await new Promise((resolve) =>
            window.setTimeout(resolve, Math.max(0, settings.retryDelay) * 1000),
          );
          if (operationRef.current !== operationId) throw new Error("本轮请求已取消");
        }
      }
    }
    throw lastError;
  };

  const executeTool = async (
    call: ToolCall,
    args: Record<string, unknown>,
    targetRegion: Region | null,
    operationId: string,
    roundId: string,
    toolStep: number,
    frameId?: string,
  ) => {
    const toolName = call.function.name;
    if (toolName === "wait") {
      const seconds = typeof args.seconds === "number" ? args.seconds : 1;
      logClientEvent("tool.wait_prepared", {
        operationId,
        roundId,
        toolStep,
        toolCallId: call.id,
        name: typeof args.name === "string" ? args.name : null,
        seconds,
      });
      setNotice(`等待 ${seconds} 秒后继续观察...`);
      return toolResultJson({ status: "waiting", seconds });
    }
    if (toolName === "end_round") {
      const message = typeof args.message === "string" ? args.message : "";
      logClientEvent("tool.round_end_requested", {
        operationId,
        roundId,
        toolStep,
        toolCallId: call.id,
        message,
      });
      return toolResultJson({ status: "round_ended" });
    }
    if (!targetRegion) throw new Error("模型请求操作，但尚未选择目标区域");
    const name = typeof args.name === "string" ? args.name : call.function.name;
    const normalizedX = typeof args.x === "number" ? args.x : null;
    const normalizedY = typeof args.y === "number" ? args.y : null;
    const imagePixel = normalizedX !== null && normalizedY !== null
      ? {
          x: Math.round(((targetRegion.width - 1) * normalizedX) / 1000),
          y: Math.round(((targetRegion.height - 1) * normalizedY) / 1000),
        }
      : null;
    logClientEvent("tool.input_prepared", {
      operationId,
      roundId,
      toolStep,
      toolCallId: call.id,
      tool: call.function.name,
      name,
      normalizedPoint: normalizedX !== null && normalizedY !== null
        ? { x: normalizedX, y: normalizedY }
        : null,
      imagePixel,
      screenPixel: imagePixel
        ? { x: targetRegion.x + imagePixel.x, y: targetRegion.y + imagePixel.y }
        : null,
      region: targetRegion,
    });
    setNotice(`正在执行 ${call.function.name}：${name}`);
    await executeInputAction(
      targetRegion,
      { type: call.function.name, ...args },
      operationId,
      roundId,
      toolStep,
      call.id,
      frameId,
    );
    return toolResultJson({ status: "input_sent" });
  };

  const runModel = async (
    conversation: Conversation,
    roundId: string,
    operationId: string,
  ) => {
    if (!settings.apiKey.trim()) {
      setPage("settings");
      throw new Error("请先在设置页填写 API Key");
    }

    const conversationId = conversation.id;
    let contextConversation = conversation;
    let currentCycleId = conversation.messages.at(-1)?.cycleId ?? uid();
    const appendCycleMessage = (message: Message) => {
      contextConversation = {
        ...contextConversation,
        updatedAt: Date.now(),
        messages: [...contextConversation.messages, message],
      };
      appendMessage(conversationId, message);
    };
    let toolCallCount = 0;
    let requestIndex = 0;
    let lastActionMarker: {
      x: number;
      y: number;
      fromX?: number;
      fromY?: number;
    } | null = null;
    let currentFrame = latestRoundFrame(conversation, roundId);
    let actionableFrameAvailable = Boolean(currentFrame);
    const availableCycleCount = new Set(
      conversation.messages
        .map((message) => message.cycleId)
        .filter((id): id is string => Boolean(id)),
    ).size;
    logClientEvent("context.prepared", {
      operationId,
      roundId,
      conversationId,
      requestedContextCycles: settings.contextCycles,
      availableCycleCount,
      hasSystemPrompt: Boolean(systemPrompt.trim()),
    });

    while (true) {
      if (operationRef.current !== operationId) throw new Error("本轮请求已取消");
      const requestTools = availableActionTools(settings.toolsEnabled, actionableFrameAvailable);
      const requestMessages = buildMessages(contextConversation, settings, systemPrompt);
      logClientEvent("round.request_prepared", {
        operationId,
        roundId,
        requestIndex: requestIndex + 1,
        toolCallCount,
        actionableFrameAvailable,
        contextCycleLimit: settings.contextCycles,
        apiMessageCount: requestMessages.length,
        offeredTools: requestTools.map((tool) => tool.function.name),
      });
      requestIndex += 1;
      setNotice(
        requestIndex === 1
          ? "模型正在观察和思考..."
          : "模型正在检查最新画面并决定下一步...",
      );
      const response = await requestWithRetry(
        requestMessages,
        operationId,
        roundId,
        requestIndex,
        requestTools,
      );
      const assistant = response.choices?.[0]?.message;
      if (!assistant) {
        throw new Error(response.error?.message || "模型没有返回有效消息");
      }

      const responseToolCalls = assistant.tool_calls ?? [];
      const offeredToolNames = new Set<string>(requestTools.map((tool) => tool.function.name));
      const computerToolAdaptation = adaptUnexpectedComputerToolCalls(
        responseToolCalls.map(ensureToolCallId),
        {
          offeredToolNames,
          frameSize: region ? { width: region.width, height: region.height } : null,
        },
      );
      for (const event of computerToolAdaptation.events) {
        logClientEvent(
          event.outcome === "adapted"
            ? "round.computer_tool_adapted"
            : "round.computer_tool_rejected",
          {
            operationId,
            roundId,
            requestIndex,
            ...event,
          },
          event.outcome === "adapted" ? "info" : "warn",
        );
      }
      const returnedToolCalls = computerToolAdaptation.calls;
      const unavailableToolCallCount = returnedToolCalls.filter(
        (call) => !offeredToolNames.has(call.function.name),
      ).length;
      if (unavailableToolCallCount > 0) {
        logClientEvent("round.unavailable_tools_returned", {
          operationId,
          roundId,
          requestIndex,
          unavailableToolCallCount,
          offeredTools: [...offeredToolNames],
        }, "warn");
      }
      const toolCalls = returnedToolCalls.slice(0, 1);
      const skippedToolCallCount = returnedToolCalls.length - toolCalls.length;
      const assistantText = extractAssistantText(assistant.content);
      if (toolCalls.length === 0) {
        const storedText = assistantText || "模型未返回最终回复。";
        appendCycleMessage({
          id: uid(),
          role: "assistant",
          roundId,
          cycleId: currentCycleId,
          kind: "final",
          text: storedText,
          createdAt: Date.now(),
        });
        logClientEvent("round.model_completed", {
          operationId,
          roundId,
          requestIndex,
          toolCallCount,
          formalReply: Boolean(assistantText.trim()),
        });
        setNotice("本轮完成");
        return Boolean(assistantText.trim());
      }

      let usedInput = false;
      let roundEnded = false;
      let roundEndMessage = "本次运行已结束。";
      let postActionDelayMs: number | null = null;
      let postActionNotice: string | null = null;
      for (const call of toolCalls) {
        if (operationRef.current !== operationId) throw new Error("本轮请求已取消");
        let toolOutcome: "succeeded" | "blocked" | "failed" = "succeeded";
        let failureCode: string | undefined;
        let failureDetail: string | undefined;
        let args: Record<string, unknown> | null = null;
        toolCallCount += 1;
        try {
          args = parseToolArguments(call);
        } catch (error) {
          toolOutcome = "failed";
          failureCode = "INVALID_TOOL_ARGUMENTS";
          failureDetail = error instanceof Error ? error.message : String(error);
        }

        const strategyAnalysis = (typeof args?.analysis === "string"
          ? args.analysis
          : "本次工具调用未提供有效打法分析，需要依据当前截图重新判断。")
          .trim()
          .slice(0, 1000);
        appendCycleMessage({
          id: uid(),
          role: "assistant",
          roundId,
          cycleId: currentCycleId,
          kind: "strategy",
          text: strategyAnalysis,
          createdAt: Date.now(),
        });

        if (args) {
          try {
            if (!settings.toolsEnabled) {
              toolOutcome = "blocked";
              failureCode = "TOOLS_DISABLED";
            } else if (!actionableFrameAvailable) {
              toolOutcome = "blocked";
              failureCode = "NO_CURRENT_FRAME";
            } else if (call.function.name === "end_round") {
              await executeTool(
                call,
                args,
                region,
                operationId,
                roundId,
                toolCallCount,
                currentFrame?.frameId,
              );
              roundEnded = true;
              roundEndMessage = typeof args.message === "string"
                ? args.message
                : roundEndMessage;
            } else {
              await executeTool(
                call,
                args,
                region,
                operationId,
                roundId,
                toolCallCount,
                currentFrame?.frameId,
              );
              usedInput = true;
              if (call.function.name === "click" || call.function.name === "hover") {
                lastActionMarker = { x: args.x as number, y: args.y as number };
              } else if (call.function.name === "drag") {
                lastActionMarker = {
                  x: args.toX as number,
                  y: args.toY as number,
                  fromX: args.fromX as number,
                  fromY: args.fromY as number,
                };
              } else if (
                call.function.name === "keyboard_type"
                || call.function.name === "keyboard_press"
              ) {
                lastActionMarker = null;
              }
              if (call.function.name === "wait") {
                postActionDelayMs = (typeof args.seconds === "number" ? args.seconds : 1) * 1000;
              } else if (call.function.name === "hover") {
                postActionDelayMs = Math.min(Math.max(1, settings.operationCaptureDelay), 3) * 1000;
                postActionNotice = "等待悬浮提示与画面稳定...";
              }
            }
          } catch (error) {
            if (isCancelledError(error) || operationRef.current !== operationId) throw error;
            toolOutcome = "failed";
            failureCode = "EXECUTION_FAILED";
            failureDetail = error instanceof Error ? error.message : String(error);
            logClientEvent(
              "tool.execution_failed",
              {
                operationId,
                roundId,
                requestIndex,
                toolStep: toolCallCount,
                toolCallId: call.id,
                tool: call.function.name,
                error: String(error),
              },
              "warn",
            );
          }
        }

        const contextText = actionResultContextText(
          call.function.name,
          toolOutcome === "succeeded" ? "success" : "failed",
          failureCode,
          failureDetail,
        );
        logClientEvent("tool.resolved", {
          operationId,
          roundId,
          requestIndex,
          toolStep: toolCallCount,
          toolCallId: call.id,
          tool: call.function.name,
          outcome: toolOutcome,
          failureCode,
          failureDetail,
          skippedToolCallCount,
        }, toolOutcome === "succeeded" ? "info" : "warn");
        appendCycleMessage({
          id: uid(),
          role: "tool",
          roundId,
          cycleId: currentCycleId,
          toolName: call.function.name,
          text: contextText,
          contextText,
          createdAt: Date.now(),
        });
      }

      if (roundEnded) {
        appendCycleMessage({
          id: uid(),
          role: "assistant",
          roundId,
          cycleId: currentCycleId,
          kind: "final",
          text: roundEndMessage,
          createdAt: Date.now(),
        });
        logClientEvent("round.model_ended", {
          operationId,
          roundId,
          requestIndex,
          toolCallCount,
        });
        setNotice("本轮完成");
        return true;
      }
      if (!region) {
        if (usedInput) throw new Error("操作后截图缺少目标区域");
        continue;
      }
      if (usedInput && !settings.autoCaptureAfterTool) {
        if (toolCalls[0]?.function.name === "wait" && postActionDelayMs !== null) {
          const delayCompleted = await sleepWithCancel(
            postActionDelayMs,
            () => operationRef.current !== operationId,
          );
          if (!delayCompleted) throw new Error("本轮请求已取消");
        }
        setNotice("工具已执行，工具后自动截图已关闭");
        return false;
      }
      if (usedInput) {
        const captureDelayMs = postActionDelayMs
          ?? Math.max(500, settings.operationCaptureDelay * 1000);
        if (postActionNotice) {
          setNotice(postActionNotice);
        } else if (postActionDelayMs === null) {
          setNotice(`等待游戏画面稳定（${settings.operationCaptureDelay} 秒）...`);
        }
        const delayCompleted = await sleepWithCancel(
          captureDelayMs,
          () => operationRef.current !== operationId,
        );
        if (!delayCompleted) throw new Error("本轮请求已取消");
      } else {
        setNotice("工具调用未执行，正在刷新当前画面...");
      }
      const rawScreenshot = await captureRegion(region, {
        operationId,
        roundId,
        captureKind: "operation-result",
        toolStep: toolCallCount,
        markerX: lastActionMarker?.x,
        markerY: lastActionMarker?.y,
        markerFromX: lastActionMarker?.fromX,
        markerFromY: lastActionMarker?.fromY,
      });
      const resultAttachment = createAttachment(
        rawScreenshot.dataUrl,
        "tool",
        region,
        rawScreenshot.frameId,
      );
      currentFrame = resultAttachment;
      const resultCreatedAt = Date.now();
      actionableFrameAvailable = true;
      currentCycleId = uid();
      appendCycleMessage({
        id: uid(),
        role: "user",
        roundId,
        cycleId: currentCycleId,
        text: "",
        createdAt: resultCreatedAt,
        attachments: [resultAttachment],
      });
    }
  };

  const sendMessage = async (override?: {
    text: string;
    attachments: Attachment[];
    conversationId?: string;
  }) => {
    if (busyRef.current) return;
    if (replyCaptureTimerRef.current !== null) {
      window.clearTimeout(replyCaptureTimerRef.current);
      replyCaptureTimerRef.current = null;
      logClientEvent("reply_capture.cancelled", { reason: "new_round_started" });
    }
    const text = override?.text ?? draft.trim();
    const attachments = override?.attachments ?? pendingAttachments;
    if (!text && attachments.length === 0) return;

    const requestedConversationId = override?.conversationId ?? activeIdRef.current;
    const baseConversation = conversationsRef.current.find(
      (conversation) => conversation.id === requestedConversationId,
    );
    if (!baseConversation) {
      logClientEvent("reply_capture.cancelled", {
        reason: "conversation_missing",
        conversationId: requestedConversationId,
      }, "warn");
      return;
    }
    const operationId = uid();
    const roundId = uid();
    const cycleId = uid();
    const trigger = override?.attachments.some((attachment) => attachment.source === "reply")
      ? "reply-capture"
      : attachments.length > 0 && !text
        ? "image"
        : attachments.length > 0
          ? "message-with-image"
          : "message";
    operationRef.current = operationId;
    roundRef.current = roundId;
    busyRef.current = true;
    setIsBusy(true);
    setIsCancelling(false);
    const conversationId = baseConversation.id;
    const userMessage: Message = {
      id: uid(),
      role: "user",
      roundId,
      cycleId,
      text,
      createdAt: Date.now(),
      attachments,
    };
    if (trigger !== "reply-capture") {
      setDraft("");
      setPendingAttachments([]);
    }

    const nextConversation = {
      ...baseConversation,
      title:
        baseConversation.messages.length === 0
          ? userMessage.text.trim().slice(0, 28) || "图片会话"
          : baseConversation.title,
      updatedAt: Date.now(),
      messages: [...baseConversation.messages, userMessage],
    };
    setConversations((current) =>
      current.map((item) => (item.id === conversationId ? nextConversation : item)),
    );

    let outcome = "completed";
    let formalReply = false;
    logClientEvent("round.started", {
      operationId,
      roundId,
      conversationId,
      trigger,
      attachmentCount: attachments.length,
      contextCycles: settings.contextCycles,
    });
    try {
      await beginOperation(operationId, roundId, trigger);
      formalReply = await runModel(nextConversation, roundId, operationId);
    } catch (error) {
      const cancelled = isCancelledError(error);
      outcome = cancelled ? "cancelled" : "failed";
      logClientEvent("round.failed", {
        operationId,
        roundId,
        conversationId,
        outcome,
        error: String(error),
      }, cancelled ? "warn" : "error");
      appendMessage(conversationId, {
        id: uid(),
        role: "assistant",
        roundId,
          text: cancelled ? `本次运行已中断：${String(error)}` : `本次运行未完成：${String(error)}`,
        createdAt: Date.now(),
      });
      setNotice(cancelled ? "本次运行已中断" : String(error));
    } finally {
      await finishOperation(operationId, roundId, outcome).catch(() => undefined);
      logClientEvent("round.finished", {
        operationId,
        roundId,
        conversationId,
        outcome,
      });
      if (operationRef.current === operationId) operationRef.current = null;
      if (roundRef.current === roundId) roundRef.current = null;
      busyRef.current = false;
      setIsBusy(false);
      setIsCancelling(false);
      if (
        outcome === "completed"
        && formalReply
        && replyCaptureEnabledRef.current
        && regionRef.current
      ) {
        const delaySeconds = Math.max(5, settings.replyCaptureDelay);
        const scheduledRegion = regionRef.current;
        logClientEvent("reply_capture.scheduled", {
          conversationId,
          completedRoundId: roundId,
          delaySeconds,
          region: scheduledRegion,
        });
        setNotice(`正式回复完成，${delaySeconds} 秒后截图开始下一轮`);
        replyCaptureTimerRef.current = window.setTimeout(() => {
          replyCaptureTimerRef.current = null;
          if (!replyCaptureEnabledRef.current || busyRef.current) return;
          const currentRegion = regionRef.current;
          if (!currentRegion) return;
          void captureRegion(currentRegion, { captureKind: "reply" })
            .then((capture) => {
              if (busyRef.current || !replyCaptureEnabledRef.current) return;
              return sendMessage({
                text: "",
                attachments: [createAttachment(capture.dataUrl, "reply", currentRegion, capture.frameId)],
                conversationId,
              });
            })
            .catch((error) => {
              setNotice(`普通回复后截图失败：${String(error)}`);
              logClientEvent("reply_capture.failed", {
                conversationId,
                completedRoundId: roundId,
                error: String(error),
              }, "error");
            });
        }, delaySeconds * 1000);
      }
    }
  };

  const interruptRun = async () => {
    const operationId = operationRef.current;
    const roundId = roundRef.current;
    if (!operationId) {
      setReplyCaptureEnabled(false);
      setNotice("已停止普通回复后截图");
      return;
    }
    setIsCancelling(true);
    operationRef.current = null;
    roundRef.current = null;
    setNotice("正在中断当前请求...");
    await cancelOperation(operationId, roundId ?? "unknown").catch(() => undefined);
  };

  const minimizeToCompanion = async () => {
    setHistoryOpen(false);
    setMiniMode(true);
    try {
      await enterMiniMode();
    } catch (error) {
      setMiniMode(false);
      setNotice(`进入迷你模式失败：${String(error)}`);
    }
  };

  const restoreFromCompanion = async () => {
    try {
      await exitMiniMode();
      setMiniMode(false);
    } catch (error) {
      setNotice(`退出迷你模式失败：${String(error)}`);
    }
  };

  const stopFromCompanion = async () => {
    setReplyCaptureEnabled(false);
    await interruptRun();
  };

  const onUpload = (event: ChangeEvent<HTMLInputElement>) => {
    const files = Array.from(event.target.files ?? []);
    files.forEach((file) => {
      const reader = new FileReader();
      reader.onload = () => {
        setPendingAttachments((current) => [
          ...current,
          createAttachment(String(reader.result), "upload"),
        ]);
        setNotice("上传图片已加入输入框，仅作为参考，不会用于键鼠坐标");
      };
      reader.readAsDataURL(file);
    });
    event.target.value = "";
  };

  const onComposerKeyDown = (event: ReactKeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      void sendMessage();
    }
  };

  const addConversation = () => {
    const conversation = newConversation(conversations.length + 1);
    setConversations((current) => [conversation, ...current]);
    setActiveId(conversation.id);
    setPage("chat");
    setHistoryOpen(false);
  };

  const deleteConversation = (id: string) => {
    if (conversations.length === 1) {
      const replacement = newConversation(1);
      setConversations([replacement]);
      setActiveId(replacement.id);
      return;
    }
    const next = conversations.filter((item) => item.id !== id);
    setConversations(next);
    if (activeId === id) setActiveId(next[0].id);
  };

  if (miniMode) {
    const statusTitle = isBusy
      ? "模型正在运行"
      : replyCaptureEnabled
        ? "普通回复后截图已开启"
        : "VLM_Screenshot_Action 待机中";
    const statusDetail = isBusy
      ? notice
      : replyCaptureEnabled
        ? `每次正式回复后等待 ${settings.replyCaptureDelay} 秒截图并开始下一轮`
        : "任务仍保留，可随时展开继续调整";
    return (
      <main className="mini-shell">
        <div className={`mini-status ${isBusy || replyCaptureEnabled ? "active" : ""}`}>
          {isBusy ? <LoaderCircle size={18} className="spin" /> : <Bot size={18} />}
          <span>
            <strong>{statusTitle}</strong>
            <small>{statusDetail}</small>
          </span>
        </div>
        <div className="mini-actions">
          {(isBusy || replyCaptureEnabled) && (
            <button className="mini-stop" onClick={() => void stopFromCompanion()} title="停止当前任务和普通回复后截图；键盘输入期间可按 F8 急停">
              <CircleStop size={16} />
            </button>
          )}
          <button className="mini-restore" onClick={() => void restoreFromCompanion()} title="恢复完整窗口">
            <Maximize2 size={16} />
            展开
          </button>
        </div>
      </main>
    );
  }

  if (page === "settings") {
    return (
      <SettingsPage
        settings={settings}
        setSettings={setSettings}
        showApiKey={showApiKey}
        setShowApiKey={setShowApiKey}
        onBack={() => setPage("chat")}
      />
    );
  }

  return (
    <main className="app-shell">
      <aside className="context-panel">
        <section className="context-card prompt-card">
          <div className="section-label"><Sparkles size={14} /> 系统提示词</div>
          <textarea
            value={systemPrompt}
            onChange={(event) => setSystemPrompt(event.target.value)}
            spellCheck={false}
          />
          <span className="field-hint">作为上下文的一部分，保存人设、偏好和游戏规则</span>
        </section>

      </aside>

      <section className="workspace">
        <header className="conversation-bar">
          <div className="conversation-nav">
            <button className="history-icon" onClick={() => setHistoryOpen(true)} title="历史记录">
              <History size={18} />
            </button>
            <button className="settings-icon" onClick={() => setPage("settings")} title="设置">
              <SettingsIcon size={18} />
            </button>
          </div>
          <div className="conversation-title">
            <h1>{activeConversation.title}</h1>
          </div>
          <div className="conversation-right">
            <button className="mini-mode-button" onClick={() => void minimizeToCompanion()} title="切换到迷你运行窗口">
              <Minimize2 size={16} />
              <span>迷你模式</span>
            </button>
            <button className="emergency-stop" onClick={() => void interruptRun()} title="键盘输入期间即使窗口隐藏，也可按 F8 急停">
              {isCancelling ? <LoaderCircle size={15} className="spin" /> : <CircleStop size={15} />}
              <span>{isBusy ? "中断运行 / F8" : "紧急停止 / F8"}</span>
            </button>
            <button className="new-conversation" onClick={addConversation} title="新建会话">
              <Plus size={21} />
            </button>
          </div>
        </header>

        <div className="chat-scroll" ref={scrollRef}>
          {activeConversation.messages.length === 0 ? (
            <EmptyState />
          ) : (
            <div className="message-list">
              {activeConversation.messages.map((message) => (
                <MessageBubble key={message.id} message={message} />
              ))}
              {isBusy && (
                <div className="thinking-row">
                  <LoaderCircle size={16} className="spin" />
                  模型正在处理画面与工具结果，可点击“中断运行”停止
                </div>
              )}
            </div>
          )}
        </div>

        <div className="capture-toolbar">
          <button
            className={replyCaptureEnabled ? "active" : ""}
            onClick={() => {
              if (!region) {
                setNotice("请先选择截图区域");
                return;
              }
              setReplyCaptureEnabled((value) => !value);
            }}
          >
            {replyCaptureEnabled ? <Pause size={15} /> : <Camera size={15} />}
            {replyCaptureEnabled ? "停止回复后截图" : "启用回复后截图"}
          </button>
          <button onClick={chooseRegion}><Crop size={15} /> 选择截图范围</button>
          <button onClick={() => void takeScreenshot()}><ImagePlus size={15} /> 手动截图</button>
        </div>

        <div className="composer-wrap">
          {pendingAttachments.length > 0 && (
            <div className="attachment-strip">
              {pendingAttachments.map((attachment) => (
                <div className="attachment-preview" key={attachment.id}>
                  <img src={attachment.dataUrl} alt="待发送截图" />
                  <button
                    onClick={() =>
                      setPendingAttachments((current) =>
                        current.filter((item) => item.id !== attachment.id),
                      )
                    }
                  >
                    <X size={12} />
                  </button>
                </div>
              ))}
            </div>
          )}
          <div className="composer">
            <button className="icon-button" onClick={() => fileInputRef.current?.click()} title="添加图片">
              <Upload size={18} />
            </button>
            <input ref={fileInputRef} hidden type="file" accept="image/*" multiple onChange={onUpload} />
            <textarea
              value={draft}
              onChange={(event) => setDraft(event.target.value)}
              onKeyDown={onComposerKeyDown}
              placeholder="图片、文字输入框"
              rows={1}
            />
            <button
              className="send-button"
              disabled={isBusy || (!draft.trim() && pendingAttachments.length === 0)}
              onClick={() => void sendMessage()}
            >
              {isBusy ? <LoaderCircle size={18} className="spin" /> : <Send size={18} />}
              发送
            </button>
          </div>
        </div>
      </section>

      {historyOpen && (
        <HistoryDrawer
          conversations={conversations}
          activeId={activeId}
          onClose={() => setHistoryOpen(false)}
          onAdd={addConversation}
          onSelect={(id) => {
            setActiveId(id);
            setHistoryOpen(false);
          }}
          onDelete={deleteConversation}
        />
      )}
    </main>
  );
}

function EmptyState() {
  return (
    <div className="empty-state">
      <div className="empty-visual">
        <div className="scan-frame"><MousePointer2 size={30} /></div>
      </div>
      <span className="eyebrow">准备开始</span>
    </div>
  );
}

const toolCallLabel = (call: ToolCall) => {
  try {
    const args = JSON.parse(call.function.arguments) as Record<string, unknown>;
    const detail = typeof args.name === "string"
      ? args.name
      : typeof args.message === "string"
        ? args.message
        : "";
    return detail ? `${call.function.name}：${detail}` : call.function.name;
  } catch {
    return call.function.name;
  }
};

function MessageBubble({ message }: { message: Message }) {
  if (message.role === "tool") return null;

  const hasToolCalls = Boolean(message.toolCalls?.length);
  if (message.role === "assistant" && hasToolCalls && !message.text.trim()) return null;

  return (
    <article className={`message ${message.role}`}>
      <div className="message-meta">
        <span>{message.role === "assistant" ? "视觉伙伴" : "你"}</span>
        <time>{new Date(message.createdAt).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}</time>
      </div>
      {message.attachments && message.attachments.length > 0 && (
        <div className="message-images">
          {message.attachments.map((attachment) => (
            <img key={attachment.id} src={attachment.dataUrl} alt="游戏截图" />
          ))}
        </div>
      )}
      {message.kind === "strategy" && <span className="strategy-label">打法分析</span>}
      {message.text.trim() && <p>{message.text}</p>}
      {hasToolCalls && (
        <div className="tool-call-chips">
          {message.toolCalls!.map((call) => (
            <span key={call.id} className="tool-chip">{toolCallLabel(call)}</span>
          ))}
        </div>
      )}
    </article>
  );
}

function HistoryDrawer({
  conversations,
  activeId,
  onClose,
  onAdd,
  onSelect,
  onDelete,
}: {
  conversations: Conversation[];
  activeId: string;
  onClose: () => void;
  onAdd: () => void;
  onSelect: (id: string) => void;
  onDelete: (id: string) => void;
}) {
  return (
    <div className="drawer-backdrop" onMouseDown={onClose}>
      <aside className="history-drawer" onMouseDown={(event) => event.stopPropagation()}>
        <div className="history-heading">
          <div><History size={17} /> 历史记录</div>
          <div className="history-heading-actions">
            <button onClick={onAdd} title="新建会话"><Plus size={18} /></button>
            <button onClick={onClose} title="关闭"><X size={18} /></button>
          </div>
        </div>
        <p className="history-intro">超出上下文的截图循环仍保留在这里；超过 15 天未更新的会话会自动删除。</p>
        <div className="history-list">
          {conversations.map((conversation) => (
            <div
              key={conversation.id}
              className={`history-item ${conversation.id === activeId ? "active" : ""}`}
            >
              <button className="history-main" onClick={() => onSelect(conversation.id)}>
                <Gamepad2 size={15} />
                <span>
                  <strong>{conversation.title}</strong>
                  <small>
                    {conversation.messages.length} 条消息 · {new Date(conversation.updatedAt).toLocaleDateString()}
                  </small>
                </span>
                <ChevronRight size={14} />
              </button>
              <button className="delete-history" onClick={() => onDelete(conversation.id)} title="删除会话">
                <Trash2 size={14} />
              </button>
            </div>
          ))}
        </div>
      </aside>
    </div>
  );
}

type SettingsPageProps = {
  settings: Settings;
  setSettings: React.Dispatch<React.SetStateAction<Settings>>;
  showApiKey: boolean;
  setShowApiKey: React.Dispatch<React.SetStateAction<boolean>>;
  onBack: () => void;
};

function SettingsPage({ settings, setSettings, showApiKey, setShowApiKey, onBack }: SettingsPageProps) {
  const patch = <K extends keyof Settings>(key: K, value: Settings[K]) =>
    setSettings((current) => ({ ...current, [key]: value }));

  return (
    <main className="settings-shell">
      <header className="settings-topbar">
        <span className="settings-top-title"><SettingsIcon size={16} /> 设置页</span>
        <button onClick={onBack}>返回首页</button>
      </header>

      <div className="settings-page">
        <div className="settings-panel">
          <section className="settings-section">
            <div className="settings-section-heading">
              <span className="eyebrow">模型连接</span>
              <strong>接口与生成参数</strong>
            </div>

            <label>
              <span>API URL</span>
              <input
                value={settings.apiUrl}
                onChange={(event) => patch("apiUrl", event.target.value)}
              />
              <small>可填写服务根地址或完整的 `/chat/completions` 地址</small>
            </label>
            <label>
              <span>API Key</span>
              <div className="secret-input">
                <input
                  type={showApiKey ? "text" : "password"}
                  value={settings.apiKey}
                  onChange={(event) => patch("apiKey", event.target.value)}
                  placeholder="sk-..."
                />
                <button onClick={() => setShowApiKey((value) => !value)} title="显示或隐藏 API Key">
                  {showApiKey ? <EyeOff size={17} /> : <Eye size={17} />}
                </button>
              </div>
            </label>
            <label>
              <span>模型名称</span>
              <input value={settings.model} onChange={(event) => patch("model", event.target.value)} />
            </label>

            <label>
              <span>思考等级预设</span>
              <div className="segmented">
                {(["low", "high", "xhigh"] as const).map((level) => (
                  <button
                    key={level}
                    className={settings.reasoningEffort === level ? "active" : ""}
                    onClick={() => patch("reasoningEffort", level)}
                  >
                    {{ low: "低", high: "高", xhigh: "XHigh" }[level]}
                  </button>
                ))}
              </div>
              <small>以 Chat Completions 顶层字段 `reasoning_effort` 发送</small>
            </label>
          </section>

          <section className="settings-section">
            <div className="settings-section-heading">
              <span className="eyebrow">运行行为</span>
              <strong>上下文、节奏与自动化</strong>
            </div>

            <div className="settings-pair">
              <label className="range-field">
                <span>最近截图循环数 <b>{settings.contextCycles}</b></span>
                <input
                  type="number"
                  min="1"
                  max="999"
                  value={settings.contextCycles}
                  onChange={(event) => patch(
                    "contextCycles",
                    Math.min(999, Math.max(1, Number(event.target.value) || 1)),
                  )}
                />
                <small>每个循环包含截图、打法分析、脱敏动作结果及更新截图；只将最近循环放入请求，完整历史仍保留在本地；范围 1-999</small>
              </label>
              <label className="range-field">
                <span>单次请求超时 <b>{settings.requestTimeout} 秒</b></span>
                <input
                  type="range"
                  min="5"
                  max="300"
                  step="5"
                  value={settings.requestTimeout}
                  onChange={(event) => patch("requestTimeout", Number(event.target.value))}
                />
                <small>达到时间后中断本次模型请求；默认 60 秒</small>
              </label>
            </div>

            <div className="settings-pair">
              <label className="range-field">
                <span>普通回复后截图 <b>{settings.replyCaptureDelay} 秒</b></span>
                <input
                  type="range"
                  min="5"
                  max="120"
                  step="5"
                  value={settings.replyCaptureDelay}
                  onChange={(event) => patch("replyCaptureDelay", Number(event.target.value))}
                />
                <small>模型正式回复结束本次运行后等待一次，再截图并创建新的运行；默认 15 秒</small>
              </label>
              <label className="range-field">
                <span>工具操作后截图 <b>{settings.operationCaptureDelay} 秒</b></span>
                <input
                  type="range"
                  min="0.5"
                  max="20"
                  step="0.5"
                  value={settings.operationCaptureDelay}
                  onChange={(event) => patch("operationCaptureDelay", Number(event.target.value))}
                />
                <small>工具操作后等待画面稳定，再发送新截图；鼠标操作可附落点或拖拽标记，默认等待 7 秒</small>
              </label>
            </div>

            <div className="settings-toggle-grid">
              <ToggleRow
                checked={settings.toolsEnabled}
                title="提供模型工具"
                detail="开启后提供鼠标、键盘、等待和结束工具；键盘输入期间按 F8 急停（窗口隐藏时也有效）"
                onToggle={() => patch("toolsEnabled", !settings.toolsEnabled)}
              />
              <ToggleRow
                checked={settings.autoCaptureAfterTool}
                title="工具后自动截图"
                detail="开启后在模型执行工具后自动截取新画面并继续运行"
                onToggle={() => patch(
                  "autoCaptureAfterTool",
                  !settings.autoCaptureAfterTool,
                )}
              />
            </div>

          </section>
        </div>
      </div>
    </main>
  );
}

function ToggleRow({
  checked,
  title,
  detail,
  onToggle,
}: {
  checked: boolean;
  title: string;
  detail: string;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      className="toggle-row"
      aria-pressed={checked}
      onClick={onToggle}
    >
      <span>
        <strong>{title}</strong>
        <small>{detail}</small>
      </span>
      <span className={`toggle ${checked ? "checked" : ""}`} aria-hidden="true">
        <i />
      </span>
    </button>
  );
}

export default App;
