import { afterEach, expect, test } from "bun:test";
import { chmodSync, copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const temporaryDirectories: string[] = [];
afterEach(() => {
  for (const directory of temporaryDirectories.splice(0)) {
    rmSync(directory, { recursive: true, force: true });
  }
});

function runBuild(mode: string, overrides: Record<string, string> = {}) {
  const directory = mkdtempSync(join(tmpdir(), "gitjuke-build-test-"));
  temporaryDirectories.push(directory);
  mkdirSync(join(directory, "scripts"));
  const script = join(directory, "scripts/build-desktop.sh");
  copyFileSync(resolve(import.meta.dir, "build-desktop.sh"), script);
  const log = join(directory, "calls");
  for (const command of ["bun", "uname", "rustup", "codesign", "xcrun", "spctl"]) {
    const path = join(directory, command);
    writeFileSync(path, `#!/bin/bash
printf '%s %s\\n' '${command}' "$*" >> "$BUILD_TEST_LOG"
case '${command}' in
  uname) echo Darwin ;;
  rustup) echo "$BUILD_TEST_BIN/cargo" ;;
  bun)
    printf 'identity=%s apple_id=%s password=%s team=%s api_key=%s certificate=%s\\n' "\${APPLE_SIGNING_IDENTITY-}" "\${APPLE_ID-}" "\${APPLE_PASSWORD-}" "\${APPLE_TEAM_ID-}" "\${APPLE_API_KEY-}" "\${APPLE_CERTIFICATE-}" >> "$BUILD_TEST_LOG"
    if [[ "$1" == run && "$2" == tauri ]]; then
      # Match Tauri's cleanup of the intermediate app for DMG-only builds.
      case " $* " in
        *" --bundles dmg "*) ;;
        *) mkdir -p target/aarch64-apple-darwin/release/bundle/macos/GitJuke.app ;;
      esac
    fi
    ;;
  codesign)
    if [[ "$1" == --verify ]]; then
      for argument in "$@"; do app_path="$argument"; done
      [[ -d "$app_path" ]] || { echo 'App removed by DMG-only bundling' >&2; exit 1; }
    fi
    if [[ "$1" == --display ]]; then echo 'TeamIdentifier=ABCDEFGHIJ'; fi
    ;;
  spctl) exit "\${BUILD_TEST_GATEKEEPER_EXIT:-0}" ;;
esac
`);
    chmodSync(path, 0o755);
  }
  const result = Bun.spawnSync(["/bin/bash", script, mode], {
    env: {
      PATH: `${directory}:/usr/bin:/bin`,
      BUILD_TEST_LOG: log,
      BUILD_TEST_BIN: directory,
      APPLE_SIGNING_IDENTITY: "Developer ID Application: Test (ABCDEFGHIJ)",
      APPLE_ID: "test@example.invalid",
      APPLE_PASSWORD: "fake-app-specific-password",
      APPLE_TEAM_ID: "ABCDEFGHIJ",
      APPLE_API_KEY: "fake-api-key",
      APPLE_CERTIFICATE: "fake-certificate",
      ...overrides,
    },
  });
  return { result, calls: readFileSync(log, "utf8") };
}

test("personal builds clear notarization credentials and use ad-hoc signing with locked dependencies", () => {
  const { result, calls } = runBuild("local");
  expect(result.exitCode).toBe(0);
  expect(calls).toContain("identity=- apple_id= password= team= api_key= certificate=");
  expect(calls).toContain("--bundles app -- --locked");
  expect(calls).toContain("codesign --verify");
  expect(calls).not.toContain("xcrun");
});

test("official releases reject absent notarization credentials and ad-hoc identities before building", () => {
  for (const overrides of [{ APPLE_PASSWORD: "" }, { APPLE_SIGNING_IDENTITY: "-" }]) {
    const { result, calls } = runBuild("release", overrides);
    expect(result.exitCode).not.toBe(0);
    expect(calls).not.toContain("bun");
  }
});

test("DMG packaging preserves the app for signature, stapling, and Gatekeeper verification", () => {
  const { result, calls } = runBuild("release");
  expect(result.exitCode).toBe(0);
  expect(calls).toContain("codesign --verify");
  expect(calls).toContain("xcrun stapler validate");
  expect(calls).toContain("spctl --assess");
});

test("official release verification checks stapling and fails if Gatekeeper rejects the app", () => {
  const { result, calls } = runBuild("release", { BUILD_TEST_GATEKEEPER_EXIT: "1" });
  expect(result.exitCode).not.toBe(0);
  expect(calls).toContain("--bundles app,dmg -- --locked");
  expect(calls).toContain("codesign --verify");
  expect(calls).toContain("codesign --display");
  expect(calls).toContain("xcrun stapler validate");
  expect(calls).toContain("spctl --assess");
});
