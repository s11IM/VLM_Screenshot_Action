import assert from "node:assert/strict";
import test from "node:test";

import {
  COMPUTER_TOOL_COORDINATE_MODE,
  adaptUnexpectedComputerToolCalls,
} from "../computerToolAdapter.ts";

const context = {
  offeredToolNames: new Set([
    "click",
    "drag",
    "hover",
    "keyboard_type",
    "keyboard_press",
    "wait",
    "end_round",
  ]),
  frameSize: { width: 1646, height: 923 },
};

const computerCall = (args) => ({
  id: "toolu_test",
  type: "function",
  function: { name: "computer", arguments: JSON.stringify(args) },
});

test("adapts a computer left click to normalized click", () => {
  const result = adaptUnexpectedComputerToolCalls([
    computerCall({ action: "left_click", coordinate: [823, 461] }),
  ], context);

  assert.equal(result.calls[0].function.name, "click");
  assert.deepEqual(JSON.parse(result.calls[0].function.arguments), {
    analysis: "Compatibility adapter mapped upstream computer.left_click to click.",
    name: "Computer click",
    x: 500,
    y: 500,
    clicks: 1,
  });
  assert.equal(result.events[0].coordinateMode, COMPUTER_TOOL_COORDINATE_MODE);
  assert.deepEqual(result.events[0].sourceCoordinates, [[823, 461]]);
});

test("adapts screenshot and wait without coordinates", () => {
  const result = adaptUnexpectedComputerToolCalls([
    computerCall({ action: "screenshot" }),
    computerCall({ action: "wait", duration: 0.25 }),
  ], context);

  assert.equal(result.calls[0].function.name, "wait");
  assert.equal(JSON.parse(result.calls[0].function.arguments).seconds, 1);
  assert.equal(result.calls[1].function.name, "wait");
  assert.equal(JSON.parse(result.calls[1].function.arguments).seconds, 1);
});

test("adapts double click, mouse move, and drag", () => {
  const result = adaptUnexpectedComputerToolCalls([
    computerCall({ action: "double_click", coordinate: [1645, 922] }),
    computerCall({ action: "mouse_move", coordinate: [0, 0] }),
    computerCall({
      action: "left_click_drag",
      start_coordinate: [0, 0],
      coordinate: [1645, 922],
    }),
  ], context);

  assert.deepEqual(
    result.calls.map((call) => call.function.name),
    ["click", "hover", "drag"],
  );
  assert.equal(JSON.parse(result.calls[0].function.arguments).clicks, 2);
  assert.deepEqual(JSON.parse(result.calls[1].function.arguments), {
    analysis: "Compatibility adapter mapped upstream computer.mouse_move to hover.",
    name: "Computer mouse move",
    x: 0,
    y: 0,
  });
  assert.deepEqual(JSON.parse(result.calls[2].function.arguments), {
    analysis: "Compatibility adapter mapped upstream computer.left_click_drag to drag.",
    name: "Computer drag",
    fromX: 0,
    fromY: 0,
    toX: 1000,
    toY: 1000,
  });
});

test("adapts computer type and key actions to keyboard tools", () => {
  const result = adaptUnexpectedComputerToolCalls([
    computerCall({ action: "type", text: "secret" }),
    computerCall({ action: "key", text: "CTRL+A" }),
  ], context);

  assert.deepEqual(result.calls.map((call) => call.function.name), [
    "keyboard_type",
    "keyboard_press",
  ]);
  assert.equal(JSON.parse(result.calls[0].function.arguments).text, "secret");
  assert.deepEqual(JSON.parse(result.calls[1].function.arguments).keys, ["CTRL", "A"]);
  assert.deepEqual(result.events.map((event) => event.outcome), ["adapted", "adapted"]);
});

test("rejects click modifiers and out-of-frame coordinates", () => {
  const result = adaptUnexpectedComputerToolCalls([
    computerCall({ action: "left_click", coordinate: [100, 100], text: "ctrl" }),
    computerCall({ action: "left_click", coordinate: [2000, 100] }),
  ], context);

  assert.deepEqual(result.calls.map((call) => call.function.name), ["computer", "computer"]);
  assert.match(result.events[0].reason, /modifiers/);
  assert.match(result.events[1].reason, /outside the current frame/);
});

test("does not adapt computer when the caller offered it", () => {
  const original = computerCall({ action: "screenshot" });
  const result = adaptUnexpectedComputerToolCalls([original], {
    ...context,
    offeredToolNames: new Set(["computer"]),
  });

  assert.equal(result.calls[0], original);
  assert.deepEqual(result.events, []);
});

test("rejects malformed computer keyboard actions", () => {
  const result = adaptUnexpectedComputerToolCalls([
    computerCall({ action: "type", text: "" }),
    computerCall({ action: "key", text: "CTRL+CTRL" }),
  ], context);

  assert.deepEqual(result.calls.map((call) => call.function.name), ["computer", "computer"]);
  assert.match(result.events[0].reason, /non-empty text/);
  assert.match(result.events[1].reason, /duplicate keys/);
});

test("cannot introduce a tool that was not offered in the current request", () => {
  const original = computerCall({ action: "left_click", coordinate: [100, 100] });
  const result = adaptUnexpectedComputerToolCalls([original], {
    ...context,
    offeredToolNames: new Set(["wait"]),
  });
  assert.equal(result.calls[0], original);
  assert.equal(result.events[0].outcome, "rejected");
  assert.match(result.events[0].reason, /not offered/);
});

test("coordinate actions need frame dimensions and unsupported actions stay rejected", () => {
  const result = adaptUnexpectedComputerToolCalls([
    computerCall({ action: "left_click", coordinate: [10, 10] }),
    computerCall({ action: "right_click", coordinate: [10, 10] }),
    { ...computerCall({}), function: { name: "computer", arguments: "{" } },
  ], { ...context, frameSize: null });
  assert.deepEqual(result.events.map((event) => event.outcome), ["rejected", "rejected", "rejected"]);
  assert.match(result.events[0].reason, /dimensions/);
  assert.match(result.events[1].reason, /not safely adaptable/);
  assert.ok(result.calls.every((call) => call.function.name === "computer"));
});
