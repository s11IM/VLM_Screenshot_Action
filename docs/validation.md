# Validation

## Automated Checks

Run from a Windows PowerShell terminal in the repository root after `npm ci`:

```powershell
npm run check
npm run fmt:rust:check
npm run build
npm run test:rust
```

`check` runs TypeScript checking and Node's test runner. Tests use synthetic
images, fake model messages, mock IPC, or recording keyboard implementations.
They do not capture the desktop, send input, or contact a model provider.

The Windows CI also scans source with a checksum-verified Gitleaks release,
checks npm advisories, and builds the release executable with locked Rust
dependencies. It does not launch the executable or publish artifacts.
The two Rust manifests are independent; testing the Tauri crate alone does
not run the dependency crate's unit tests.

## Contract Coverage

| Contract | Tests |
| --- | --- |
| Current round screenshot, never an uploaded reference | `src/__tests__/model.test.mjs` |
| Context window, active task, summary, and one-image retention | `src/__tests__/model.test.mjs` |
| Tool availability and argument validation | `src/__tests__/model.test.mjs` |
| Upstream frame-pixel conversion and safe rejection | `src/__tests__/computerToolAdapter.test.mjs` |
| Frame token carried across IPC | `src/__tests__/tauri.test.mjs` |
| Public package privacy gate and core-library rebuild detection | `src/__tests__/scripts.test.mjs` (temporary fixtures only) |
| Coordinate edges and release of keys after cancellation | `desktop-core/src/lib.rs` tests |
| Keyboard frame lifetime, foreground binding, DPI mapping | `src-tauri/src/lib.rs` tests |

These checks do not prove that a game accepts injected input, that a model
obeys the prompt, or that the React orchestration is race-free. The complete
UI loop and provider compatibility still need integration/manual validation.

## Source Hygiene

With Gitleaks installed, scan source without printing matched secrets:

```powershell
gitleaks dir . --config .gitleaks.toml --redact --no-banner
```

The configuration excludes generated dependencies and build outputs only.
It does not certify packaged archives; inspect those separately. After commits
exist, also use `gitleaks git . --redact --no-banner` to check history. A clean
scan is a useful check, not proof that no sensitive information exists.

## Supervised Smoke Test

Use a harmless test application and a non-sensitive desktop, never a terminal,
payment page, privileged application, or account with valuable state.

- [ ] A fresh checkout installs and builds without copied caches or credentials.
- [ ] Framework window opens on Windows 10/11 x64 with WebView2 installed.
- [ ] Region selection and Escape cancellation restore the main window.
- [ ] Selection/click align at 100%, 125%, and 150% scaling where available.
- [ ] Secondary monitors and negative desktop origins map correctly.
- [ ] Manual screenshot excludes the app window and displays correctly.
- [ ] Sending text or an uploaded reference alone does not offer action tools.
- [ ] A click/hover/drag executes once, then a fresh image is observed.
- [ ] Disabling tool-result capture prevents the next model iteration after input.
- [ ] Keyboard input requires a fresh matching frame; switching focus interrupts it.
- [ ] Holding F8 during a long key hold stops it and releases pressed keys.
- [ ] UI cancellation works during a pending request and a settling delay.
- [ ] Reply capture starts a new round only when enabled; stopping it cancels the timer.
- [ ] A tool failure is distinguishable from verified success in subsequent context.
- [ ] Restart restores history/settings but does not resume an operation.

Record the model/provider, monitor layout, Windows version, and tested commit.
Never describe a check as passed merely because corresponding code exists.
