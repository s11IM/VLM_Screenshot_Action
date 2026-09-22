# Core Design

VLM_Screenshot_Action is an experimental Windows reference implementation of a
visual feedback loop. The reusable idea is not a game-specific strategy: it is
how to connect observations, bounded actions, feedback, and cancellation.
The application contains no model weights, game-state reader, or training code.

## Reading Map

| Read in order | Responsibility |
| --- | --- |
| [`src/types.ts`](../src/types.ts) | Conversation, message, attachment, and tool contracts |
| [`src/model.ts`](../src/model.ts) | Prompt, tools, context selection, image retention, argument validation |
| [`src/App.tsx`](../src/App.tsx) | `sendMessage`, `runModel`, `executeTool`, and automatic observation |
| [`src/tauri.ts`](../src/tauri.ts) | Typed calls across the JavaScript/Rust boundary |
| [`src-tauri/src/lib.rs`](../src-tauri/src/lib.rs) | Native commands, frame validation, request cancellation, window handling |
| [`desktop-core/src/lib.rs`](../desktop-core/src/lib.rs) | Physical coordinates and interruptible desktop input |
| [`src/computerToolAdapter.ts`](../src/computerToolAdapter.ts) | Explicit compatibility conversion for unexpected upstream tools |

The controller currently lives in React, not in a background Rust agent.
Rust provides native operations and enforces selected safety checks. There is
no local HTTP server or MCP server in this repository.

## Observe, Decide, Act, Verify

```mermaid
sequenceDiagram
    participant User
    participant Controller as React controller
    participant Native as Tauri / Rust
    participant Model as Configured model API
    participant Desktop
    User->>Controller: Select region, capture, send a task
    Controller->>Native: begin_operation(operationId)
    loop Until final reply, interruption, or error
        Controller->>Controller: Select recent cycles; retain one image
        Controller->>Native: request_chat_completion(context, tools)
        Native->>Model: POST /chat/completions
        Model-->>Controller: Reply or tool calls (via Rust)
        Controller->>Controller: Adapt, select first call, validate arguments
        opt Mouse or keyboard action
            Controller->>Native: execute_input_action(region, frameId, action)
            Native->>Desktop: Dispatch input after checks
        end
        Note over Controller,Desktop: Input dispatch is not proof of task success
        opt Continue with automatic observation
            Controller->>Native: capture_region after settling delay
            Native->>Desktop: Capture selected region
            Native-->>Controller: PNG data URL and frameId
        end
    end
    Controller->>Native: finish_operation
```

1. Selection establishes a physical screen rectangle. Sending text alone does
   not capture a starting frame.
2. `sendMessage` records the user message and starts an operation.
3. `runModel` rebuilds context for each request. Tools are offered only when
   enabled and a non-upload frame exists in the current round.
4. A response without tool calls is treated as a final reply. `end_round`
   explicitly ends the current round.
5. At most the first returned tool call is executed, even if the provider
   ignores `parallel_tool_calls: false`.
6. `wait` delays observation without native input. Mouse and keyboard tools
   cross the IPC boundary. Ordinary failures become an action-result message
   and, when a region exists, a refreshed observation.
7. With tool-result capture enabled, successful input is followed by a delay,
   a new screenshot, and another model request. With it disabled, a successful
   action ends the run without another observation.

The implementation uses a non-streaming Chat Completions-compatible API.
`reasoning_effort` is always sent; not every provider or model supports it.
Request timeout and one optional retry limit individual network requests,
not the total number of actions or total cost.

## Identity and Lifetime

| Identity | Meaning and lifetime |
| --- | --- |
| Conversation ID | Persisted history across multiple runs |
| `roundId` | One user send or reply-triggered send; may contain many requests |
| `operationId` | Native cancellation state for that run |
| `cycleId` | Observation and subsequent analysis/result group used for context selection |
| `frameId` | Native screenshot token; keyboard validation consumes the current token |

A result screenshot starts the next cycle. An operation is not a conversation,
and an ended round is not necessarily the end of automatic observation.

## Context Is a Projection, Not the History

The IndexedDB history retains complete messages and image data. `buildMessages`
creates a smaller request view:

- The fixed system contract is combined with user-supplied instructions.
- Only the latest `contextCycles` groups are selected (default: three).
- The latest nonempty user task is restored as `[ACTIVE_TASK]` when outside
  that window.
- The latest eligible final summary after that task can be restored as
  `[LAST_ROUND_SUMMARY]`.
- Strategy text and compact `[ACTION_RESULT]` feedback are included. Historical
  tool calls are not replayed as a full assistant/tool protocol transcript;
  action feedback is sent as user-role text.
- `expireOldImages(messages, 1)` retains only the last image across the request.
  Earlier image payloads are removed and their labels become `OMITTED`.
- Only a retained non-upload screenshot can become `CURRENT_FRAME`. Uploaded
  references never become actionable frames and share the same image budget.

This reduces payload size and stale visual grounding. It trades away visual
comparison of old frames; text history cannot replace missing visual evidence.
The helper functions are pure enough to test without an API or desktop.

## Coordinates and Feedback

Native model tools use inclusive integer coordinates in `0..1000`:

```text
screenX = region.x + round((region.width  - 1) * x / 1000)
screenY = region.y + round((region.height - 1) * y / 1000)
```

The last pixel, not the pixel beyond the rectangle, maps to 1000. Selection
converts WebView logical coordinates using the actual window geometry.
Windows mouse movement uses the virtual desktop to handle secondary monitors
and negative origins. A selected region must fit within one monitor.

Capture temporarily hides the application window. Result images may include
a crosshair or drag trail showing the previous action. These are annotations
on captured pixels, not overlays installed in the target application.

The compatibility adapter accepts a limited set of unexpected `computer`
actions. Its coordinates are explicitly interpreted as **frame pixels**, then
converted to `0..1000`. It rejects unsupported actions, click modifiers,
out-of-frame points, and mappings to tools not offered in that request. This
is a compatibility assumption, not automatic detection of a provider's units.

## Execution and Cancellation

The native keyboard gate requires the current `frameId`, the same region,
a frame younger than five minutes, and the same nonzero foreground window
recorded during a stable capture. A successful gate consumes the token.
Mouse actions invalidate the stored keyboard frame but do not use the same
frame-age/foreground gate.

Keyboard input checks cancellation, foreground changes, and F8 while running.
Once tripped, the operation stays cancelled. Pressed keys are released in
reverse order; a drag attempts to release its mouse button on failure.
F8 is not a global emergency shortcut during network requests or mouse input.

UI cancellation invalidates the frontend operation and sets a native atomic
flag. HTTP work races that flag against timeout; native input checks it at
short intervals. Cancellation is cooperative: already dispatched input cannot
be undone, and not every capture or delay is instantly interruptible.

## Two Observation Loops

| Loop | Trigger | Identity |
| --- | --- | --- |
| Tool-result capture | After an action settles (normally seven seconds) | New cycle, same round |
| Reply capture | After a formal reply, when explicitly enabled (normally fifteen seconds) | New round and operation |

Hover uses a shorter one-to-three-second delay. `wait` uses its requested
one-to-sixty-second delay. Reply capture is completion-triggered, not a fixed
interval screenshot service. `end_round` does not disable reply capture.

## Boundaries and Non-Goals

- This is not a sandbox, unattended production agent, or guaranteed game bot.
- Window movement or another window covering the selected rectangle can change
  what mouse input affects. Keyboard focus checks do not restrict OS shortcuts.
- Screen text and model output are untrusted. Tool validation limits argument
  shapes; it does not establish that an action is appropriate or safe.
- There is no total step/time/cost budget or general human approval gate yet.
- App shutdown stops orchestration; persisted history does not resume a run.
- There is no supported Linux/macOS/mobile build despite some portable code.

See [security boundaries](../SECURITY.md), [validation](validation.md), and
[contribution scope](../CONTRIBUTING.md). Prefer preserving these contracts
over turning this reference into a general multi-agent framework.

## Repository Conventions Studied

The repository organization borrows conventions, not implementation, from:

- [Tauri architecture](https://github.com/tauri-apps/tauri/blob/dev/ARCHITECTURE.md):
  describe components and the development/release flow separately.
- [Tauri contribution guide](https://github.com/tauri-apps/tauri/blob/dev/.github/CONTRIBUTING.md):
  map contributions to components and require reproducible, reviewed changes.
- [UI-TARS Desktop contribution guide](https://github.com/bytedance/UI-TARS-desktop/blob/main/CONTRIBUTING.md):
  separate development validation from maintainer-controlled releases.
- [UI-TARS Desktop security policy](https://github.com/bytedance/UI-TARS-desktop/blob/main/SECURITY.md):
  keep vulnerability reporting distinct from ordinary bug reports.

These references are not endorsements. This project deliberately omits their
monorepo release machinery, CLA processes, and broad platform matrices.
