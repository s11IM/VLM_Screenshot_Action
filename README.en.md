# VLM_Screenshot_Action

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Platform: Windows](https://img.shields.io/badge/Platform-Windows-0078D6.svg)](https://www.microsoft.com/windows)
[![Tauri](https://img.shields.io/badge/Tauri-2.x-FFC131.svg)](https://tauri.app)
[![Windows Validation](https://github.com/s11IM/VLM_Screenshot_Action/actions/workflows/ci.yml/badge.svg)](https://github.com/s11IM/VLM_Screenshot_Action/actions/workflows/ci.yml)

[中文](README.md) | **English**

A lightweight vision-language model (VLM) screenshot action tool: select a screen region, the AI understands the frame, drives mouse and keyboard with normalized coordinates, and loops until the task is done. No API or accessibility interface is required from the target application.

## Design Decisions and Tradeoffs

### 1. Pure vision + normalized coordinates, not UIA / DOM / accessibility APIs

- **Gain**: works across any software — games, emulators, and legacy apps alike; no interface required from the target program
- **Cost**: targeting accuracy is bounded by the model's vision (small buttons and low-contrast UI are hard); cannot control programs running with higher privileges (UAC)
- **Normalized coordinate space**: the model locates points on a 0–1000 relative plane (top-left `0,0`, bottom-right `1000,1000`), decoupled from resolution, window size, and DPI

### 2. Rolling context + single-frame upload, not accumulating every screenshot

- **Problem**: screenshots are token-expensive; accumulating them per round quickly blows the context window and budget
- **Approach**: keep only the most recent N observe-act cycles (`contextCycles`, 1–999); each request carries only the latest screenshot while historical ones are replaced with an `[IMAGE frame=OMITTED]` placeholder — token cost does not grow with rounds
- **Compensation**: information scrolled out of the window is not simply dropped — the active task (`ACTIVE_TASK`) and the previous round summary (`LAST_ROUND_SUMMARY`) are preserved as text anchors, so long tasks do not lose the goal
- **Cost**: the model loses long-term visual memory and can only fall back to text anchors; tasks that require looking back at much older frames do not fit

### 3. Mandatory per-step analysis, not bare tool calls

Every tool call must carry an `analysis` of ≤1000 characters (the reasoning for this step). It serves two purposes at once: an auditable decision trail, and the raw material for the summaries that survive the rolling window in decision 2.

### 4. Per-step self-calibration closes the feedback loop

The latest screenshot is overlaid with a red cross / drag trajectory marking the previous landing point. The model corrects drift from the gap between "expected landing point" and "actual screen change" — a mitigation for the cost accepted in decision 1.

### 5. Safety boundaries as a first-class design

| Mechanism | Effect |
|-----------|--------|
| F8 global panic stop | Holding F8 during keyboard input or while waiting for the screen interrupts immediately; the stop latches and never auto-resumes. Other phases (network request, full-auto interval, click/drag/hover) need the on-screen stop button |
| Keyboard safety lock | Keyboard actions require frameId, capture region, and foreground window to all match the latest capture; a capture older than 5 minutes or a changed foreground window aborts safely |
| Auto window evasion | The main window hides itself before capture and before actions — no occlusion, no focus stealing |
| Redacted logging | JSONL logs redact sensitive fields by name, such as API keys, screenshot base64, and conversation text (`[redacted]`); 2MB rotation ×4 archives. Redaction matches a field-name list and cannot cover arbitrary error strings, so review logs before sharing |
| Cancellation propagation | An interrupt cascades to in-flight API requests, input actions, and idle-loop timers; combo keys release in reverse order |

See [SECURITY.md](SECURITY.md) for details.

### 6. Tauri monolith + restrained dependencies

About 6,800 lines of source today (TypeScript ≈ 3,200 + Rust ≈ 3,600), no Electron / Node.js runtime bundling, a standard NSIS installer. The frontend is just React + lucide icons; the Rust side is enigo + xcap + reqwest with no framework-level abstraction — the core logic stays readable and portable.

### Tool protocol

Seven OpenAI function-calling tools (click / drag / hover / keyboard_type / keyboard_press / wait / end_round); unexpected Anthropic-style `computer` tool calls are safely mapped automatically (pixel coordinates converted to 0–1000; unmappable actions are rejected).

---

## Architecture

### Core Flow

```mermaid
flowchart TD
    A([You take a screenshot by hand, write the task, then send<br/>with full auto on, the timer also starts rounds on its own]) --> B[Screenshot the target region and send it along with the task<br/>only the newest image travels, older ones become a placeholder line]
    B --> C{The AI looks at the image: what should it do next?}
    C -- replies with text, calls no tool --> R[Store that text in the conversation<br/>the round ends here]
    C -- calls end_round to say the round is over --> R
    C -- calls one action tool --> D{Can this step run right now?}
    D -- the tools toggle is off, or there is no usable image yet --> D2[This step did not happen<br/>the reason goes back to the AI]
    D -- the action is not allowed, or it broke while running --> D3[This step failed<br/>the reason goes back to the AI]
    D2 --> I2[Take a fresh screenshot right away, no waiting]
    D3 --> I2
    I2 --> I3{Three steps in a row that could not run?}
    I3 -- yes --> I4([End the round on its own and explain why in the conversation])
    I3 -- no --> J
    D -- yes --> E[Act: click, drag, hover, type, press a key, or wait a while longer]
    E --> E2[Remember where it acted and mark that spot with a red dot<br/>typing and key presses get no red dot]
    E2 --> H{Is auto-screenshot after tools on?}
    H -- no, and this step was not a wait --> H2([The round stops; it waits for you to capture by hand])
    H -- yes --> I[Hide the window first and wait for the screen to change, then settle<br/>capture early once it settles, or at the latest when the time is up]
    I --> J[The new screenshot becomes the starting point of the next cycle]
    J --> C
    R --> K{Is full auto on?}
    K -- yes --> L[Wait the set interval, then an auto-screenshot starts the next round] --> B
    K -- no --> M([Stop and wait for your next instruction])
    N[Hold F8 during keyboard input or while waiting for the screen: stop at once<br/>use the on-screen stop button in any other phase] -.-> C
```

The whole app simply makes the AI repeat a "look → act → look again to verify" loop until the task is done or you stop it. Each step sends only the latest screenshot (older ones become a one-line placeholder) to control cost; the internals are explained in the design decisions above.

Details that the diagram leaves out but the code really does:

- **One action per turn.** The model sometimes returns several tool calls at once; only the first is executed and the rest are dropped.
- **Three steps in a row that cannot run end the round by themselves.** Steps that could not execute (tools off, no usable frame, invalid arguments, an error while running) add up; a single success in between clears the count, so ordinary runs are never cut short. Before this guard existed, a model that kept returning calls that could not run would loop forever (`toolFailureGuard.ts`).
- **Failed steps do not consume the wait.** A step that was not executed or failed never starts the wait timer; the app grabs a fresh screenshot right away and asks the model again (`App.tsx:959-979`). Only an action that truly ran goes on to "wait for the screen to settle", and only that successful path clears the remainder left over from an early wake (`App.tsx:834-835`).
- **A `wait` after an early wake resumes from the pause point.** If the screen changes and then settles during the wait, the model is woken early and the remaining time is stored; if it then picks `wait`, that remaining time is used to keep waiting (with no further probing), while any other action restarts the full timer (`observationTimer.ts:3`).
- **Turning off early probing** falls back to a plain timer: sleep the configured time and take one screenshot, with no early wake.
- **A brand-new round needs a screenshot of its own before the model gets any tools.** Only three attachment sources count as an actionable frame: manual capture, full auto, and the capture taken after a tool ran; an image you upload from a file is reference only and never counts (`model.ts:248-253`). So a first round where you only selected a region and typed a message offers no tools at all and the model can only reply with text; the tools appear once a later round carries a capture.
- **A failed request is retried once** (3 s apart by default), with the request timeout configurable (60 s by default).
- **With no region selected at all**, no tools are offered to the model, so it can only reply with text; if it returns a tool call anyway, that call is recorded as not executed and the loop goes straight back to the model without a screenshot, since there is no region to capture (`App.tsx:950-953`).

### Module Map

```
┌────────────────────────── Frontend (React + TS) ──────────────────────────┐
│  App.tsx                  session/loop state machine, capture-act loop,   │
│                            retry and cancellation                          │
│  model.ts                 tool schemas, rolling-context assembly,          │
│                            single-frame expiry, argument validation       │
│  computerToolAdapter.ts   Anthropic computer tool calls → project tools   │
│  storage.ts               IndexedDB session persistence (15-day GC)       │
└──────────────────────────┬─────────────────────────────────────────────────┘
                        Tauri invoke
┌──────────────────────────▼─────────────────────────────────────────────────┐
│  src-tauri/lib.rs          capture (auto-hides main window), API relay     │
│                            (cancellable/timeout), keyboard safety lock,   │
│                            region selection, mini mode, redacted logging  │
│  desktop-core/             input primitives: SendInput absolute coords     │
│                            (virtual desktop), key parsing, interruptible  │
│                            hold/typing, DPI awareness                     │
└───────────────────────────────────────────────────────────────────────────┘
```

See [docs/architecture.md](docs/architecture.md) for module responsibilities and a reading order.

---

## Quick Start

### Prerequisites

- Windows 10 / 11
- Node.js ≥ 24.12 and a stable Rust toolchain (for building)
- An API key for any OpenAI-compatible endpoint (the model must support vision)

### Build

```bash
git clone https://github.com/s11IM/VLM_Screenshot_Action.git
cd VLM_Screenshot_Action
npm ci
npm run tauri dev     # development
npm run tauri build   # installers land in src-tauri/target/release/bundle/
```

### First Run

1. **Configure the API**: open settings via the gear icon (top right), fill in API URL, API key, and model name
2. **Select a region**: click "Select capture region" and drag a rectangle over the target window on the screen overlay; Esc cancels
3. **Give a task**: describe the goal in the input box (e.g. "click the start button and open settings") and send
4. **Watch the loop**: the AI runs the capture → analyze → act → re-capture loop and can be interrupted at any time; for unattended runs enable "screenshot after reply" and switch to mini mode

---

## Tool Set

| Tool | Purpose | Key parameters |
|------|---------|----------------|
| `click` | Single / double click | `x, y` (0–1000), `clicks: 1\|2` |
| `drag` | Drag | `fromX, fromY, toX, toY` |
| `hover` | Hover (wait for tooltip) | `x, y` |
| `keyboard_type` | Type literal Unicode text | `text` (≤10000 chars), `intervalMs`, `submit` |
| `keyboard_press` | Physical keys / combos | `keys[]` (1–8), `holdMs` |
| `wait` | Wait for the screen to change | none (no duration to specify) |
| `end_round` | End this run | `message` (summary for the user) |

---

## Configuration

| Setting | Default | Range / Notes |
|---------|---------|---------------|
| API URL | `https://api.openai.com/v1` | any OpenAI-compatible endpoint |
| Model | `gpt-4o-mini` | must support vision and function calling |
| Reasoning Effort | `low` | `low` / `high` / `xhigh`, passed through |
| Context cycles | `3` | 1–999, rolling window size |
| Delay after action | `8` s | 0.5–120 s, wait for a stable frame |
| Idle interval | `15` s | 5–120 s, full-auto loop interval |
| Request timeout | `60` s | 5–300 s |
| Retry on failure | on, 3 s apart | 0–15 s, at most one retry |
| Enable tools | on | off = conversation only |
| Auto screenshot after tool | on | off = manual screenshots drive the next round |

---

## Compatibility Notes

- The API layer depends only on the OpenAI-compatible `/chat/completions` format — OpenAI, Qwen, Zhipu GLM, DeepSeek, and aggregator gateways all work directly
- Some models (via compatibility gateways) return Anthropic-style `computer` tool calls; the adapter maps them safely:
  `screenshot→wait`, `left_click/double_click→click`, `mouse_move→hover`, `left_click_drag→drag`, `type→keyboard_type`, `key→keyboard_press`, with pixel coordinates converted to 0–1000 normalized; actions that cannot be mapped safely are rejected
- Multi-monitor and mixed-DPI setups are supported (including negative-coordinate secondary screens); the capture region must lie entirely within one monitor

## Known Limitations

1. Windows only (input and foreground-window detection rely on Win32 APIs)
2. Cannot control programs running with higher privileges (UAC boundary)
3. Exclusive fullscreen games should be switched to windowed / borderless
4. Targeting accuracy for small buttons and low-contrast UI depends on the model's vision
5. `keyboard_type` sends characters one by one — not suited for fast entry of very long texts

---

## Development

```bash
npm run check          # TS typecheck + frontend unit tests
npm run test:rust      # cargo test for desktop-core and src-tauri
npm run fmt:rust:check # Rust formatting check
npm run tauri dev      # dev mode
npm run tauri build    # build the NSIS installer
```

Runtime logs live in `vlm_screenshot_action.log` under the system application-log directory.

## License

[MIT](LICENSE)
