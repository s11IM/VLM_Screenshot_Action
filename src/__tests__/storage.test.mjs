import assert from "node:assert/strict";
import test from "node:test";
import { hydrateConversations, planConversationSave } from "../storage.ts";

const attachment = (id, dataUrl) => ({
  id,
  dataUrl,
  source: "tool",
  capturedAt: 1,
});

const conversation = (messages) => ({
  id: "c1",
  title: "会话",
  createdAt: 1,
  updatedAt: 2,
  messages,
});

test("a save carries only payloads the store does not have yet", () => {
  const conversations = [conversation([
    { id: "m1", role: "user", text: "", createdAt: 1, attachments: [
      attachment("a1", "data:image/png;base64,AAA"),
      attachment("a2", "data:image/png;base64,BBB"),
    ] },
  ])];

  const first = planConversationSave(conversations, new Set());
  assert.deepEqual(first.imagesToWrite.map((image) => image.id), ["a1", "a2"]);
  // The persisted record must not carry the payload again.
  assert.deepEqual(
    first.stripped[0].messages[0].attachments.map((item) => item.dataUrl),
    ["", ""],
  );
  assert.deepEqual([...first.referencedImageIds].sort(), ["a1", "a2"]);

  // Second save: nothing new to write even though the payloads are still in memory.
  const second = planConversationSave(conversations, new Set(["a1", "a2"]));
  assert.deepEqual(second.imagesToWrite, []);
  assert.deepEqual([...second.referencedImageIds].sort(), ["a1", "a2"]);
});

test("planning never mutates the in-memory conversations", () => {
  const conversations = [conversation([
    { id: "m1", role: "user", text: "", createdAt: 1, attachments: [
      attachment("a1", "data:image/png;base64,AAA"),
    ] },
  ])];

  planConversationSave(conversations, new Set());
  // Rendering and request building still need the real payload.
  assert.equal(conversations[0].messages[0].attachments[0].dataUrl, "data:image/png;base64,AAA");
});

test("hydration restores payloads and drops attachments whose image is gone", () => {
  const stored = [conversation([
    { id: "m1", role: "user", text: "", createdAt: 1, attachments: [
      attachment("a1", ""),
      attachment("missing", ""),
    ] },
  ])];

  const hydrated = hydrateConversations(stored, new Map([["a1", "data:image/png;base64,AAA"]]));
  const attachments = hydrated[0].messages[0].attachments;
  assert.equal(attachments.length, 1);
  assert.equal(attachments[0].id, "a1");
  assert.equal(attachments[0].dataUrl, "data:image/png;base64,AAA");
});

test("hydration keeps legacy inline payloads as they are", () => {
  const stored = [conversation([
    { id: "m1", role: "user", text: "", createdAt: 1, attachments: [
      attachment("a1", "data:image/png;base64,OLD"),
    ] },
  ])];

  const hydrated = hydrateConversations(stored, new Map());
  assert.equal(hydrated[0].messages[0].attachments[0].dataUrl, "data:image/png;base64,OLD");
});

test("messages without attachments are passed through untouched", () => {
  const message = { id: "m1", role: "assistant", text: "hi", createdAt: 1 };
  const conversations = [conversation([message])];

  const plan = planConversationSave(conversations, new Set());
  assert.equal(plan.stripped[0].messages[0], message);
  assert.equal(hydrateConversations(conversations, new Map())[0].messages[0], message);
});

test("a repeated attachment id is queued for writing once", () => {
  const shared = "data:image/png;base64,SHARED";
  const conversations = [conversation([
    { id: "m1", role: "user", text: "", createdAt: 1, attachments: [attachment("a1", shared)] },
    { id: "m2", role: "user", text: "", createdAt: 2, attachments: [attachment("a1", shared)] },
  ])];

  const plan = planConversationSave(conversations, new Set());
  assert.deepEqual(plan.imagesToWrite.map((image) => image.id), ["a1"]);
  assert.deepEqual([...plan.referencedImageIds], ["a1"]);
});
