"""Regression checks for test selection and success-cache correctness."""
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("rust_runner", Path(__file__).with_name("test-rust.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


class RustTasks(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.paths = {"model": "crates/model/", "remote": "crates/remote/", "core": "crates/core/", "desktop": "desktop/"}
        self.dependencies = {"model": set(), "remote": {"model"}, "core": {"remote", "model"}, "desktop": {"core"}}
        self.git("init", "-q")
        self.write(".gitignore", "target/\n")
        self.write("Cargo.lock", "locked\n")
        self.write(".config/nextest.toml", "configuration\n")
        for prefix in self.paths.values():
            self.write(prefix + "src/lib.rs", "original\n")
        self.git("add", ".")
        self.git("-c", "user.name=Test", "-c", "user.email=test@example.invalid",
                 "-c", "commit.gpgsign=false", "-c", "core.hooksPath=" + str(self.root / "no-hooks"),
                 "commit", "-qm", "initial")

    def git(self, *args):
        return subprocess.check_output(["git", *args], cwd=self.root, stderr=subprocess.STDOUT)

    def write(self, path, text):
        file = self.root / path
        file.parent.mkdir(parents=True, exist_ok=True)
        file.write_text(text)

    def key(self, name="core", suite="fast", context=None):
        return runner.task_key(self.root, name, suite, self.paths, self.dependencies, context or {})

    def test_changes_select_dependents_but_not_dependencies(self):
        self.assertEqual(runner.affected_packages({"crates/remote/src/lib.rs"}, self.paths, self.dependencies), {"remote", "core", "desktop"})
        self.assertEqual(runner.affected_packages({"desktop/src/lib.rs"}, self.paths, self.dependencies), {"desktop"})
        self.assertEqual(runner.affected_packages(set(), self.paths, self.dependencies), set())

    def test_unknown_and_global_inputs_select_every_package(self):
        for file in ["Cargo.lock", ".cargo/config.toml", "scripts/new-helper.py", "src/App.tsx", "removed-crate/src/lib.rs"]:
            with self.subTest(file=file):
                self.assertEqual(runner.affected_packages({file}, self.paths, self.dependencies), set(self.paths))

    def test_staged_unstaged_new_and_deleted_paths_are_detected(self):
        self.write("crates/model/src/lib.rs", "staged")
        self.git("add", "crates/model/src/lib.rs")
        self.write("crates/remote/src/lib.rs", "unstaged")
        self.write("crates/core/tests/new.json", "new fixture")
        (self.root / "desktop/src/lib.rs").unlink()
        self.assertEqual(runner.changed_files(self.root, "HEAD"), {
            "crates/model/src/lib.rs", "crates/remote/src/lib.rs", "crates/core/tests/new.json", "desktop/src/lib.rs",
        })

    def test_transitive_fixture_and_deletion_invalidate_cache(self):
        original = self.key()
        self.write("crates/model/src/lib.rs", "changed dependency")
        self.assertNotEqual(original, self.key())
        previous = self.key()
        self.write("crates/remote/tests/fixture.json", "new untracked fixture")
        self.assertNotEqual(previous, self.key())
        previous = self.key()
        (self.root / "crates/remote/src/lib.rs").unlink()
        self.assertNotEqual(previous, self.key())

    def test_unrelated_code_and_build_output_do_not_invalidate_cache(self):
        original = self.key("model")
        self.write("crates/core/src/lib.rs", "unrelated dependent change")
        self.write("target/something", "new build output")
        self.assertEqual(original, self.key("model"))

    def test_configuration_toolchain_environment_and_suite_invalidate_cache(self):
        original = self.key()
        self.assertNotEqual(original, self.key(suite="full"))
        self.assertNotEqual(original, self.key(context={"rustc": "different compiler"}))
        self.assertNotEqual(original, self.key(context={"environment": {"GITJUKE_TEST_HOOK": "1"}}))
        self.write(".config/nextest.toml", "changed test selection")
        self.assertNotEqual(original, self.key())

    def test_executable_mode_is_part_of_the_inputs(self):
        path = "crates/core/tests/helper.sh"
        self.write(path, "exit 0")
        original = self.key()
        (self.root / path).chmod(0o755)
        self.assertNotEqual(original, self.key())

    def test_environment_filters_credentials_but_tracks_test_hooks(self):
        self.assertEqual(runner.relevant_environment({
            "GH_TOKEN": "secret", "GITHUB_RUN_ID": "123", "GITJUKE_TEST_HOOK": "hook", "RUSTFLAGS": "flags",
        }), {"GITJUKE_TEST_HOOK": "hook", "RUSTFLAGS": "flags"})

    def execute(self, key, code, use_cache=True, after=None, docs=None):
        return runner.execute(self.root, "core", "fast", key, lambda: after or key,
                              self.root / "target/cache", use_cache, [sys.executable, "-c", code], docs)

    def test_only_successful_complete_tasks_are_cached(self):
        code = "from pathlib import Path; p=Path('calls'); p.write_text(p.read_text()+'x' if p.exists() else 'x')"
        self.assertEqual(self.execute("success", code), 0)
        self.assertEqual(self.execute("success", code), 0)
        self.assertEqual((self.root / "calls").read_text(), "x")
        self.assertEqual(self.execute("failure", "raise SystemExit(7)"), 7)
        self.assertFalse((self.root / "target/cache/failure.json").exists())
        self.assertEqual(self.execute("docs", "pass", docs=[sys.executable, "-c", "raise SystemExit(8)"]), 8)
        self.assertFalse((self.root / "target/cache/docs.json").exists())

    def test_changes_during_execution_are_not_cached(self):
        self.assertEqual(self.execute("before", "pass", after="after"), 0)
        self.assertFalse((self.root / "target/cache/before.json").exists())

    def test_force_run_bypasses_cache_and_failure_removes_old_success(self):
        self.assertEqual(self.execute("key", "pass"), 0)
        self.assertEqual(self.execute("key", "raise SystemExit(9)", use_cache=False), 9)
        self.assertFalse((self.root / "target/cache/key.json").exists())

    def test_corrupt_and_incomplete_cache_entries_are_misses(self):
        stamp = self.root / "stamp.json"
        for content in ["invalid", "{}", json.dumps({"schema": 1, "key": "a", "passed": False})]:
            stamp.write_text(content)
            self.assertFalse(runner.cache_hit(stamp, "a"))


if __name__ == "__main__":
    unittest.main()
