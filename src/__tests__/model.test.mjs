import assert from "node:assert/strict";
import test from "node:test";

import {
  ACTION_TOOLS,
  SYSTEM_PROMPT,
  availableActionTools,
  buildMessages,
  expireOldImages,
  latestRoundFrame,
  parseToolArguments,
} from "../model.ts";

const call = (name, args) => ({
  id: `toolu_${name}`,
  type: "function",
  function: { name, arguments: JSON.stringify(args) },
});

const common = {
  analysis: "依据当前画面执行下一步。",
  name: "测试动作",
};

const frame = (id, source = "tool") => ({
  id,
  source,
  dataUrl: `data:image/png;base64,${id}`,
  capturedAt: 0,
  frameId: source === "upload" ? undefined : id,
});

test("tools require both explicit enablement and a current observation", () => {
  assert.deepEqual(availableActionTools(false, true), []);
  assert.deepEqual(availableActionTools(true, false), []);
  assert.equal(availableActionTools(true, true), ACTION_TOOLS);
});

test("context selection preserves a cropped task and latest eligible summary", () => {
  const conversation = { messages: [
    { role: "user", cycleId: "task", text: "Reach the exit" },
    { role: "assistant", cycleId: "task", kind: "strategy", text: "Old reasoning" },
    { role: "assistant", cycleId: "summary", kind: "final", text: "Door unlocked" },
    { role: "user", cycleId: "current", text: "", attachments: [frame("current")] },
    { role: "assistant", cycleId: "current", kind: "strategy", text: "Approach the door" },
    { role: "tool", cycleId: "current", text: "raw result", contextText: "[ACTION_RESULT tool=click status=SUCCESS]" },
  ] };
  const before = structuredClone(conversation);
  const messages = buildMessages(conversation, { contextCycles: 1 }, "Avoid shortcuts");

  assert.ok(messages[0].content.startsWith(SYSTEM_PROMPT));
  assert.ok(messages[0].content.endsWith("Avoid shortcuts"));
  assert.deepEqual(messages.slice(1, 3), [
    { role: "user", content: "[ACTIVE_TASK]\nReach the exit" },
    { role: "assistant", content: "[LAST_ROUND_SUMMARY]\nDoor unlocked" },
  ]);
  assert.equal(messages[4].content, "[STRATEGY_ANALYSIS]\nApproach the door");
  assert.deepEqual(messages[5], {
    role: "user", content: "[ACTION_RESULT tool=click status=SUCCESS]",
  });
  assert.equal(messages.length, 6);
  assert.equal(messages.some((message) => message.role === "tool" || message.tool_calls), false);
  assert.deepEqual(conversation, before);
});

test("a new task does not inherit an older task's final summary", () => {
  const messages = buildMessages({ messages: [
    { role: "user", cycleId: "old", text: "Old task" },
    { role: "assistant", cycleId: "old", kind: "final", text: "Old summary" },
    { role: "user", cycleId: "new", text: "New task" },
    { role: "assistant", cycleId: "new", kind: "strategy", text: "New strategy" },
  ] }, { contextCycles: 1 }, "");

  assert.deepEqual(messages, [
    { role: "system", content: SYSTEM_PROMPT },
    { role: "user", content: "New task" },
    { role: "assistant", content: "[STRATEGY_ANALYSIS]\nNew strategy" },
  ]);
});

test("a retained task and final summary appear once without extra anchors", () => {
  const messages = buildMessages({ messages: [
    { role: "user", cycleId: "current", text: "Task" },
    { role: "assistant", cycleId: "current", kind: "final", text: "Summary" },
  ] }, { contextCycles: 3 }, "");

  assert.deepEqual(messages.slice(1), [
    { role: "user", content: "Task" },
    { role: "assistant", content: "[LAST_ROUND_SUMMARY]\nSummary" },
  ]);
});

test("recent cycles preserve text but the request carries only the latest image", () => {
  const messages = buildMessages({ messages: [
    { role: "user", cycleId: "old", text: "Task", attachments: [frame("old", "manual")] },
    { role: "assistant", cycleId: "old", kind: "strategy", text: "Previous analysis" },
    { role: "user", cycleId: "new", text: "", attachments: [frame("new"), frame("reference", "upload")] },
  ] }, { contextCycles: 2 }, "");
  const before = structuredClone(messages);
  const prepared = expireOldImages(messages, 1);
  const parts = prepared.flatMap((message) => Array.isArray(message.content) ? message.content : []);

  assert.deepEqual(parts.filter((part) => part.type === "image_url").map((part) => part.image_url.url), [frame("new").dataUrl]);
  assert.equal(parts.filter((part) => part.text === "[IMAGE frame=OMITTED]").length, 2);
  assert.equal(parts.filter((part) => part.text?.includes("frame=CURRENT_FRAME")).length, 1);
  assert.ok(prepared.some((message) => message.content === "[STRATEGY_ANALYSIS]\nPrevious analysis"));
  assert.deepEqual(messages, before);
});

test("an uploaded reference never becomes CURRENT_FRAME, even when it is last", () => {
  const conversation = { messages: [
    { role: "user", cycleId: "old", text: "", attachments: [frame("old")] },
    { role: "user", cycleId: "new", text: "Reference", attachments: [frame("reference", "upload")] },
  ] };
  const prepared = expireOldImages(buildMessages(conversation, { contextCycles: 2 }, ""));
  const parts = prepared.flatMap((message) => Array.isArray(message.content) ? message.content : []);
  assert.equal(parts.some((part) => part.text?.includes("frame=CURRENT_FRAME")), false);
  assert.equal(parts.filter((part) => part.type === "image_url").length, 1);
  assert.ok(parts.some((part) => part.text?.includes("frame=REFERENCE_IMAGE")));
});

test("a zero image budget removes images without modifying history", () => {
  const messages = buildMessages({ messages: [
    { role: "user", cycleId: "current", text: "Task", attachments: [frame("current")] },
  ] }, { contextCycles: 1 }, "");
  const prepared = expireOldImages(messages, 0);
  assert.equal(prepared[1].content.some((part) => part.type === "image_url"), false);
  assert.equal(messages[1].content.some((part) => part.type === "image_url"), true);
});

test("malformed JSON, unsupported tools, and out-of-range coordinates are rejected", () => {
  assert.throws(() => parseToolArguments({ function: { name: "click", arguments: "{" } }));
  assert.throws(() => parseToolArguments(call("shell", { command: "anything" })));
  for (const x of [-1, 1001, null]) {
    assert.throws(() => parseToolArguments(call("click", { ...common, x, y: 500, clicks: 1 })));
  }
  assert.equal(parseToolArguments(call("click", { ...common, x: 1000, y: 0, clicks: 1 })).x, 1000);
});

test("uses the latest screenshot in this round, not a reference or previous round", () => {
  const old = { source: "manual", frameId: "old" };
  const latest = { source: "tool", frameId: "current" };
  const conversation = { messages: [
    { roundId: "previous", attachments: [old] },
    { roundId: "current", attachments: [old, latest, { source: "upload" }] },
  ] };
  assert.equal(latestRoundFrame(conversation, "current"), latest);
  assert.equal(latestRoundFrame(conversation, "missing"), undefined);
});

test("legacy screenshots cannot inherit a newer frame token", () => {
  const legacy = { source: "manual" };
  const conversation = { messages: [
    { roundId: "current", attachments: [{ source: "manual", frameId: "older" }, legacy] },
  ] };
  assert.equal(latestRoundFrame(conversation, "current"), legacy);
  assert.equal(latestRoundFrame(conversation, "current").frameId, undefined);
});

test("offers keyboard tools with mouse tools", () => {
  assert.deepEqual(
    ACTION_TOOLS.map((tool) => tool.function.name),
    ["click", "drag", "hover", "keyboard_type", "keyboard_press", "wait", "end_round"],
  );
});

test("parses Unicode typing defaults and optional submit", () => {
  assert.deepEqual(parseToolArguments(call("keyboard_type", {
    ...common,
    text: "中文\nnext",
  })), {
    ...common,
    text: "中文\nnext",
    intervalMs: 0,
    submit: false,
  });

  assert.deepEqual(parseToolArguments(call("keyboard_type", {
    ...common,
    text: "ready",
    intervalMs: 25,
    submit: true,
  })).submit, true);
});

test("parses physical keys and rejects invalid keyboard arguments", () => {
  assert.deepEqual(parseToolArguments(call("keyboard_press", {
    ...common,
    keys: ["CTRL", "A"],
  })), {
    ...common,
    keys: ["CTRL", "A"],
    holdMs: 50,
  });

  assert.throws(
    () => parseToolArguments(call("keyboard_type", { ...common, text: "" })),
    /text 必须是字符串/,
  );
  assert.throws(
    () => parseToolArguments(call("keyboard_press", { ...common, keys: ["CTRL", "ctrl"] })),
    /不允许重复按键/,
  );
});
