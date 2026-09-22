import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, rmSync, utimesSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const root = fileURLToPath(new URL("../../", import.meta.url));
const windows = process.platform === "win32";
const runScript = (path, args = []) => spawnSync("powershell.exe", [
  "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", path, ...args,
], { encoding: "utf8", timeout: 15000 });

test("public packaging rejects settings and history before any build or copy", { skip: !windows }, () => {
  for (const flag of ["-IncludeSettings", "-IncludeHistory"]) {
    const result = runScript(join(root, "scripts", "package-portable.ps1"), ["-PublicRelease", flag, "-SkipBuild"]);
    assert.ifError(result.error);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /Public releases cannot include settings or history/);
  }
});

test("release freshness includes desktop-core sources and toolchain inputs", { skip: !windows }, () => {
  const fixture = mkdtempSync(join(tmpdir(), "vlm-release-test-"));
  try {
    mkdirSync(join(fixture, "scripts"));
    const script = join(fixture, "scripts", "release-stale.ps1");
    copyFileSync(join(root, "scripts", "release-stale.ps1"), script);
    const executable = join(fixture, "app.exe");
    writeFileSync(executable, "test fixture, never executed");
    const buildTime = new Date("2026-01-01T12:00:00Z");
    const before = new Date("2026-01-01T11:00:00Z");
    const after = new Date("2026-01-01T13:00:00Z");
    utimesSync(executable, buildTime, buildTime);
    const inputs = [
      "desktop-core/src/lib.rs",
      "desktop-core/Cargo.toml",
      "desktop-core/Cargo.lock",
      "rust-toolchain.toml",
      ".node-version",
    ];
    for (const relative of inputs) {
      const path = join(fixture, relative);
      mkdirSync(dirname(path), { recursive: true });
      writeFileSync(path, "fixture");
      utimesSync(path, before, before);
    }
    const fresh = runScript(script, ["-AppPath", executable]);
    assert.ifError(fresh.error);
    assert.equal(fresh.status, 0, fresh.stderr);
    for (const relative of inputs) {
      const path = join(fixture, relative);
      utimesSync(path, after, after);
      const stale = runScript(script, ["-AppPath", executable]);
      assert.ifError(stale.error);
      assert.equal(stale.status, 1, `${relative}: ${stale.stderr}`);
      utimesSync(path, before, before);
    }
  } finally {
    rmSync(fixture, { recursive: true, force: true });
  }
});

test("public archive contains the license and no inherited user profile", { skip: !windows }, () => {
  const fixture = mkdtempSync(join(tmpdir(), "vlm-package-test-"));
  try {
    const scripts = join(fixture, "scripts");
    mkdirSync(scripts);
    for (const name of ["package-portable.ps1", "release-stale.ps1", "portable-start.cmd"]) {
      copyFileSync(join(root, "scripts", name), join(scripts, name));
    }
    copyFileSync(join(root, "LICENSE"), join(fixture, "LICENSE"));
    writeFileSync(join(scripts, "launch-app.ps1"), "param([string]$AppPath, [switch]$ValidateOnly)\nif (-not $ValidateOnly) { throw 'Do not launch test fixtures' }\n");
    const release = join(fixture, "src-tauri", "target", "release");
    const bundle = join(release, "bundle", "nsis");
    mkdirSync(bundle, { recursive: true });
    const app = join(release, "vlm_screenshot_action.exe");
    const installer = join(bundle, "test-x64-setup.exe");
    writeFileSync(app, "not an executable");
    writeFileSync(installer, "not an installer");
    const timestamp = new Date("2026-01-01T12:00:00Z");
    utimesSync(app, timestamp, timestamp);
    utimesSync(installer, timestamp, timestamp);
    const output = join(fixture, "release", "public");
    const staging = join(output, "VLM_Screenshot_Action-Portable-x64");
    mkdirSync(join(staging, "user-data"), { recursive: true });
    writeFileSync(join(staging, "user-data", "private.txt"), "synthetic private fixture");

    const result = runScript(join(scripts, "package-portable.ps1"), ["-PublicRelease", "-SkipBuild", "-ArchiveOnly"]);
    assert.ifError(result.error);
    assert.equal(result.status, 0, result.stderr);
    assert.equal(existsSync(staging), false);
    assert.ok(existsSync(join(output, "VLM_Screenshot_Action-Setup-x64.exe")));
    const listing = spawnSync("tar.exe", ["-tf", join(output, "VLM_Screenshot_Action-Portable-x64.zip")], { encoding: "utf8", timeout: 15000 });
    assert.ifError(listing.error);
    assert.equal(listing.status, 0, listing.stderr);
    assert.match(listing.stdout, /LICENSE/);
    assert.match(listing.stdout, /VLM_Screenshot_Action\.exe/);
    assert.doesNotMatch(listing.stdout, /user-data|private\.txt/);
  } finally {
    rmSync(fixture, { recursive: true, force: true });
  }
});
