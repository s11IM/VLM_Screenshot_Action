export type Region = {
  x: number;
  y: number;
  width: number;
  height: number;
};

export type Settings = {
  apiUrl: string;
  apiKey: string;
  model: string;
  reasoningEffort: "low" | "high" | "xhigh";
  contextCycles: number;
  replyCaptureDelay: number;
  operationCaptureDelay: number;
  requestTimeout: number;
  retryDelay: number;
  retryOnFailure: boolean;
  toolsEnabled: boolean;
  autoCaptureAfterTool: boolean;
};

export type Attachment = {
  id: string;
  dataUrl: string;
  width?: number;
  height?: number;
  source: "manual" | "reply" | "upload" | "tool";
  capturedAt?: number;
  frameId?: string;
};

export type CapturedImage = {
  dataUrl: string;
  frameId: string;
};

export type Message = {
  id: string;
  role: "user" | "assistant" | "tool" | "system";
  text: string;
  createdAt: number;
  roundId?: string;
  cycleId?: string;
  attachments?: Attachment[];
  toolCalls?: ToolCall[];
  toolCallId?: string;
  toolName?: string;
  contextText?: string;
  kind?: "strategy" | "final";
};

export type Conversation = {
  id: string;
  title: string;
  createdAt: number;
  updatedAt: number;
  messages: Message[];
};

export type ChatCompletionMessage = {
  role: "system" | "user" | "assistant" | "tool";
  content?: string | Array<Record<string, unknown>> | null;
  tool_calls?: ToolCall[];
  tool_call_id?: string;
};

export type ToolCall = {
  id: string;
  type: "function";
  function: {
    name: string;
    arguments: string;
  };
};

export type ChatCompletionResponse = {
  choices?: Array<{
    message?: ChatCompletionMessage;
    finish_reason?: string;
  }>;
  error?: { message?: string };
};
