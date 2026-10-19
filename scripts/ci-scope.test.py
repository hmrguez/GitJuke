"""Exercise CI scope and gate safety without running Rust tests or desktop builds."""
import importlib.util
import itertools
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).with_name("ci-scope.py")
spec = importlib.util.spec_from_file_location("ci_scope", SCRIPT)
scope = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scope)


class ScopeTests(unittest.TestCase):
    def test_frontend_and_docs_skip_native(self):
        self.assertFalse(scope.native_required([
            "src/Sidebar.tsx", "src/ui-copy.ts", "src/nested/component.css",
            "public/icon.svg", "index.html", "scripts/sidebar.test.tsx",
            "scripts/diff.test.mjs", "tests/frontend/sidebar.test.tsx", "tests/frontend/diff.test.mjs", "README.md", "AGENTS.md", "docs/development.md",
        ]))
        self.assertFalse(scope.native_required([]))

    def test_rust_shared_contract_configuration_and_unknown_paths_keep_native(self):
        for path in [
            "crates/gitjuke-core/src/runtime.rs", "src-tauri/src/lib.rs", "src/api.ts", "src/lib/ipc.ts", "src/lib/types.ts", "src/embedded.rs",
            "Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "package.json", "bun.lock",
            "tsconfig.json", "vite.config.ts", "bunfig.toml", ".gitignore",
            ".config/nextest.toml", ".cargo/config.toml", ".github/workflows/ci.yml",
            "src-tauri/capabilities/main.json", "src-tauri/icons/icon.png",
            "scripts/test-rust.py", "scripts/ci-scope.py", "scripts/build-desktop.sh",
            "tests/tooling/build-desktop.test.ts", "tests/tooling/release-eligibility.test.ts",
            "scripts/build-desktop.test.ts", "scripts/fixtures/tauri.reader-qualification.json",
            "new-directory/unknown-file",
        ]:
            with self.subTest(path=path):
                self.assertTrue(scope.native_required(["src/Sidebar.tsx", path]))

    def test_releases_and_reusable_workflows_force_native_without_a_diff(self):
        self.assertTrue(scope.detect(Path("/missing"), "push", {}, "refs/tags/v1.0.0")[0])
        self.assertTrue(scope.detect(Path("/missing"), "pull_request", {}, force_full=True)[0])

    def test_unavailable_ranges_fail_closed(self):
        for event_name, event in [
            ("pull_request", {}), ("push", {"before": "0" * 40, "after": "a" * 40}),
            ("push", {"before": "a" * 40, "after": "b" * 40}), ("unknown", {}),
        ]:
            with self.subTest(event=event):
                self.assertTrue(scope.detect(Path("/missing"), event_name, event)[0])

    def test_gate_only_accepts_required_successes_and_intentional_skips(self):
        statuses = ["success", "failure", "cancelled", "skipped", ""]
        for changes, frontend, native, rust, desktop in itertools.product(statuses, statuses, ["true", "false", ""], statuses, statuses):
            expected = (changes, frontend, native, rust, desktop) in {
                ("success", "success", "true", "success", "success"),
                ("success", "success", "false", "skipped", "skipped"),
            }
            self.assertEqual(scope.gate_passes(changes, frontend, native, rust, desktop), expected)

    def test_gate_command_preserves_exit_status(self):
        env = {**os.environ, "CHANGES_RESULT": "success", "FRONTEND_RESULT": "success",
               "NATIVE_REQUIRED": "false", "RUST_RESULT": "skipped", "DESKTOP_RESULT": "skipped"}
        command = [sys.executable, str(SCRIPT), "gate"]
        self.assertEqual(subprocess.run(command, env=env, capture_output=True).returncode, 0)
        env["FRONTEND_RESULT"] = "failure"
        self.assertEqual(subprocess.run(command, env=env, capture_output=True).returncode, 1)


class GitRanges(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.git("init", "-q", "-b", "master")
        self.write("src/Label.tsx", "initial")
        self.write("crates/core/src/lib.rs", "initial")
        self.base = self.commit()

    def git(self, *args):
        return subprocess.check_output(["git", *args], cwd=self.root, stderr=subprocess.PIPE).decode().strip()

    def write(self, path, content):
        file = self.root / path
        file.parent.mkdir(parents=True, exist_ok=True)
        file.write_text(content)

    def commit(self):
        self.git("add", "-A")
        self.git("-c", "user.name=Test", "-c", "user.email=test@example.invalid",
                 "-c", "commit.gpgsign=false", "-c", "core.hooksPath=" + str(self.root / "no-hooks"),
                 "commit", "-qm", "fixture")
        return self.git("rev-parse", "HEAD")

    def test_frontend_push_then_native_push(self):
        self.write("src/Label.tsx", "label edit")
        frontend = self.commit()
        self.assertFalse(scope.detect(self.root, "push", {"before": self.base, "after": frontend})[0])
        self.write("crates/core/src/lib.rs", "rust edit")
        native = self.commit()
        self.assertTrue(scope.detect(self.root, "push", {"before": frontend, "after": native})[0])

    def test_pr_uses_merge_base_when_target_has_advanced(self):
        self.git("checkout", "-qb", "frontend")
        self.write("src/Label.tsx", "label edit")
        head = self.commit()
        self.git("checkout", "-q", "master")
        self.write("crates/core/src/lib.rs", "unrelated target-branch edit")
        base = self.commit()
        event = {"pull_request": {"base": {"sha": base}, "head": {"sha": head}}}
        self.assertEqual(scope.changed_paths(self.root, "pull_request", event), ["src/Label.tsx"])
        self.assertFalse(scope.detect(self.root, "pull_request", event)[0])

    def test_deletions_and_both_sides_of_renames_are_classified(self):
        (self.root / "crates/core/src/lib.rs").rename(self.root / "src/moved.ts")
        head = self.commit()
        paths = scope.changed_paths(self.root, "push", {"before": self.base, "after": head})
        self.assertIn("crates/core/src/lib.rs", paths)
        self.assertIn("src/moved.ts", paths)
        self.assertTrue(scope.native_required(paths))

    def test_detect_command_writes_boolean_job_output(self):
        self.write("src/Label.tsx", "label edit")
        head = self.commit()
        event = self.root / "event.json"
        event.write_text(json.dumps({"before": self.base, "after": head}))
        output = self.root / "outputs"
        self.write("scripts/ci-scope.py", SCRIPT.read_text())
        command = [sys.executable, str(self.root / "scripts/ci-scope.py"), "detect"]
        env = {**os.environ, "GITHUB_EVENT_PATH": str(event), "GITHUB_EVENT_NAME": "push",
               "GITHUB_REF": "refs/heads/master", "GITHUB_OUTPUT": str(output), "CI_FORCE_FULL": "false"}
        result = subprocess.run(command, env=env, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(output.read_text(), "native_required=false\n")
        output.unlink()
        env["CI_FORCE_FULL"] = "true"
        result = subprocess.run(command, env=env, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(output.read_text(), "native_required=true\n")


if __name__ == "__main__":
    unittest.main()
