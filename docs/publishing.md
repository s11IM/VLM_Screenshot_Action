# Publishing Checklist

The first public release should be a readable, reproducible Windows reference
implementation. The project owner's README is intentionally separate from
these engineering documents. Nothing in this checklist publishes automatically.

## Before the First Public Push

- [ ] Finish the root README and link to architecture, contribution, security,
  and validation documents. Describe experimental status and Windows-only support.
- [ ] Confirm the MIT copyright attribution and permission to distribute all
  source, icons, and demonstration media. Dependencies retain their own licenses.
- [ ] After choosing a GitHub owner/name, add real repository metadata to npm
  and Cargo manifests. Do not ship invented repository URLs or contact addresses.
- [ ] Inspect intended files in the local Git repository before the first commit. Keep
  lockfiles; omit dependencies, build outputs, generated schemas, and private data.
- [ ] Run a secret scanner on intended content; inspect screenshots and logs
  manually. Once history exists, scan history as well as the working tree.
- [ ] Complete a clean Windows build and the checks in `validation.md`.
- [ ] Enable private vulnerability reporting in repository settings, plus
  available secret scanning/push protection and dependency alerts.
- [ ] Run GitHub CI, then require its checks for the default branch. Keep
  Actions tokens read-only unless an explicitly reviewed workflow needs more.

## Source Versus Binary Releases

The Git repository contains source, documentation, tests, and build inputs.
An installer or portable ZIP belongs in GitHub Releases, not source history.
CI intentionally does not create releases or attach executables.

Build a public package from a clean checkout:

```powershell
npm ci
npm run check
npm run test:rust
npm run package:public
```

`package:public` rejects settings/history inclusion and uses a separate
`release/public` output directory. It packages the project license and checks
that no `user-data` directory was included. This is a guardrail, not a substitute
for inspecting the final ZIP or satisfying third-party binary license notices.

`package-migration.cmd` is for personal migration. Never upload a migration
archive made with `-IncludeSettings` or `-IncludeHistory`; it can contain API
keys, prompts, chat history, and screenshots. Cache cleanup does not sanitize
existing archives or personal WebView profiles.

## Before Attaching Binaries

- [ ] Keep application versions aligned in `package.json`, its lockfile,
  `src-tauri/Cargo.toml`, and `src-tauri/tauri.conf.json`. The core library version
  may remain independent.
- [ ] Generate/review dependency license notices for the actual bundled npm
  and Rust dependencies and include notices required by their licenses.
- [ ] Inspect the archive for profiles, keys, screenshots, logs, and local paths.
- [ ] Run the supervised smoke checks and list actual known limitations.
- [ ] Publish checksums; distinguish unsigned builds from signed releases.
- [ ] Record the tested commit/tag, toolchain versions, and build commands.

Project-level MIT licensing does not replace third-party notices or guarantee
rights to third-party game imagery. A source-only initial release is valid
while binary distribution checks remain incomplete.
