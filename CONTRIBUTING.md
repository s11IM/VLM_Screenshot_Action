# Contributing

This project shares a small, runnable visual-control reference implementation.
Prioritize clear contracts, deterministic tests, and safe failure over a larger
feature surface. Start with [the architecture](docs/architecture.md).

## Windows Development

- Windows 10/11 x64 and Microsoft Edge WebView2 Runtime.
- Node.js 24 LTS; `.node-version` selects the CI baseline.
- Rust MSVC through rustup; `rust-toolchain.toml` selects the compiler.
- Visual Studio 2022 Build Tools with Desktop development with C++ and a
  Windows SDK. See [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/).

From a PowerShell terminal at the repository root:

```powershell
npm ci
npm run check
npm run build
npm run test:rust
```

Use a Visual Studio developer terminal if Cargo cannot find the linker or SDK.
The release build helper imports the Visual Studio environment automatically:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-release.ps1
```

To run the desktop UI during development, use `npm run tauri -- dev`.
`npm run dev` alone is only a browser UI preview, not a desktop-input runtime.
`clickstart.cmd` builds when necessary and launches the release application.
The executable is located at `src-tauri/target/release/vlm_screenshot_action.exe`;
`clickstart.cmd` prints its absolute path before launching.
`cleancache.cmd` removes generated dependencies/builds, not personal WebView data.

API credentials are not needed for tests or builds. Configure your own endpoint
and key in the application only for supervised manual testing. Never use real
desktop input or paid model requests in automated tests.

## Change Scope

- Keep one behavioral change per pull request; explain the invariant it changes.
- Add regression tests for context selection, coordinates, tool translation,
  or cancellation when those contracts change.
- Keep platform input in `desktop-core`, native validation/IPC in `src-tauri`,
  and model context/protocol logic in `src/model.ts` or the adapter.
- Do not introduce a generic framework or split modules only for aesthetics.
- Keep all three dependency lockfiles. Update them deliberately, not incidentally.
- Use `npm run fmt:rust` for Rust formatting and `npm run fmt:rust:check` to verify.
- Review and test AI-assisted changes yourself. Contributors are responsible
  for the code and claims they submit.
- Be respectful and discuss technical tradeoffs rather than people.

See [validation](docs/validation.md) for automated and manual checks. Include
commands actually run and any untested platform or behavior in the PR.

## Issues and Security

For bugs, include versions, sanitized reproduction steps, expected/actual
behavior, and whether the issue concerns capture, API, input, or UI. Never
attach API keys, personal screenshots, raw profiles, or unreviewed logs.
Report vulnerabilities using [SECURITY.md](SECURITY.md), not public bug details.

## Licensing and Releases

Contributions are under the repository's [MIT license](LICENSE). Retain relevant
third-party notices; do not submit code or artwork you lack permission to share.
Releases are a separate maintainer step described in [publishing](docs/publishing.md).
CI validates code but does not upload installers, publish packages, or deploy.
