#!/usr/bin/env python3
"""Run Rust test tasks with transitive content caching and conservative selection."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import stat
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parent.parent
CACHE_SCHEMA = 1


def capture(command, root=ROOT):
    return subprocess.check_output(command, cwd=root, text=True).strip()


def workspace(root):
    metadata = json.loads(capture([
        "cargo", "metadata", "--format-version", "1", "--no-deps", "--locked", "--offline",
    ], root))
    members = set(metadata["workspace_members"])
    packages = {
        p["name"]: p for p in metadata["packages"] if p["id"] in members
    }
    locations = {str(Path(p["manifest_path"]).parent): name for name, p in packages.items()}
    dependencies = {
        name: {locations[d["path"]] for d in p["dependencies"] if d.get("path") in locations}
        for name, p in packages.items()
    }
    return packages, dependencies


def closure(names, edges):
    result = set(names)
    pending = list(names)
    while pending:
        for name in edges[pending.pop()]:
            if name not in result:
                result.add(name)
                pending.append(name)
    return result


def repository_files(root):
    # Includes staged, unstaged, and new fixtures; respects ignored build output.
    output = subprocess.check_output([
        "git", "ls-files", "-z", "--cached", "--others", "--exclude-standard",
    ], cwd=root)
    return sorted(set(os.fsdecode(p) for p in output.split(b"\0") if p))


def changed_files(root, base):
    revision = capture(["git", "rev-parse", "--verify", "--end-of-options", base + "^{commit}"], root)
    changed = subprocess.check_output([
        "git", "diff", "--name-only", "--no-renames", "-z", revision, "--",
    ], cwd=root)
    untracked = subprocess.check_output([
        "git", "ls-files", "--others", "--exclude-standard", "-z",
    ], cwd=root)
    return {os.fsdecode(p) for p in (changed + untracked).split(b"\0") if p}


def package_paths(root, packages):
    return {
        name: Path(p["manifest_path"]).parent.relative_to(root).as_posix() + "/"
        for name, p in packages.items()
    }


def affected_packages(changed, paths, dependencies):
    direct = set()
    for path in changed:
        owners = {name for name, prefix in paths.items() if path.startswith(prefix)}
        if not owners:
            # Includes shared manifests, tooling, and desktop build-script inputs.
            # Unknown inputs must never silently drop a dependent test task.
            return set(paths)
        direct.update(owners)
    reverse = {name: set() for name in paths}
    for name, deps in dependencies.items():
        for dep in deps:
            reverse[dep].add(name)
    return closure(direct, reverse)


def relevant_environment(environment):
    exact = {
        "RUSTUP_TOOLCHAIN", "RUSTC", "RUSTDOC", "RUSTFLAGS", "RUSTDOCFLAGS",
        "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "CARGO_ENCODED_RUSTFLAGS",
        "CARGO_ENCODED_RUSTDOCFLAGS", "CARGO_INCREMENTAL", "CC", "CXX", "CFLAGS", "CXXFLAGS",
        "AR", "SDKROOT", "MACOSX_DEPLOYMENT_TARGET", "DEVELOPER_DIR",
        "PATH", "LANG", "LC_ALL", "TZ", "NEXTEST_TEST_THREADS",
    }
    prefixes = ("GITJUKE_", "CARGO_BUILD_", "CARGO_TARGET_", "CARGO_PROFILE_", "NEXTEST_", "DYLD_", "LD_")
    return {key: value for key, value in environment.items() if key in exact or key.startswith(prefixes)}


def task_key(root, name, suite, paths, dependencies, context):
    sources = closure({name}, dependencies)
    digest = hashlib.sha256()
    digest.update(json.dumps({
        "schema": CACHE_SCHEMA, "package": name, "suite": suite, "context": context,
    }, sort_keys=True).encode())
    for path in repository_files(root):
        shared = (
            path in {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "rust-toolchain", ".gitignore"}
            or path.endswith("/Cargo.toml")
            or path.startswith((".cargo/", ".config/", ".github/workflows/"))
            or path in {"scripts/test-rust.py", "scripts/setup-nextest.sh"}
        )
        if not shared and not any(path.startswith(paths[source]) for source in sources):
            continue
        file = root / path
        digest.update(path.encode() + b"\0")
        if not file.exists():
            digest.update(b"deleted\0")
            continue
        digest.update(str(stat.S_IMODE(file.stat().st_mode)).encode() + b"\0")
        digest.update(hashlib.sha256(file.read_bytes()).digest())
    return digest.hexdigest()


def cache_hit(path, key):
    try:
        return json.loads(path.read_text()) == {"schema": CACHE_SCHEMA, "key": key, "passed": True}
    except (OSError, ValueError):
        return False


def run_task(root, command):
    print("+ " + " ".join(str(part) for part in command), flush=True)
    return subprocess.run(command, cwd=root).returncode


def selection_args(suite):
    profile = "default" if suite == "fast" else suite
    args = ["--profile", profile, "--user-config-file", "none"]
    if suite == "qualification":
        args += ["--ignore-default-filter", "-E", "not default()"]
    return args


def check_partition(root, nextest):
    selections = {}
    for suite in ["fast", "qualification", "full"]:
        data = json.loads(capture([
            str(nextest), "nextest", "list", "--workspace", "--locked", "--offline",
            "--message-format", "json", *selection_args(suite),
        ], root))
        selections[suite] = {
            (binary, name)
            for binary, tests in data["rust-suites"].items()
            for name, test in tests["testcases"].items()
            if not test["ignored"] and test["filter-match"]["status"] == "matches"
        }
    fast, qualification, full = (selections[suite] for suite in ["fast", "qualification", "full"])
    if fast & qualification or fast | qualification != full:
        raise RuntimeError("Fast and qualification suites must be disjoint and cover the full enabled suite")
    print(f"Partition verified: {len(fast)} fast + {len(qualification)} qualification = {len(full)} enabled tests", flush=True)


def execute(root, name, suite, key, key_after, cache_dir, use_cache, command, doc_command=None):
    stamp = cache_dir / (key + ".json")
    if use_cache and cache_hit(stamp, key):
        print(f"HIT  {name} ({suite})", flush=True)
        return 0
    print(f"RUN  {name} ({suite})", flush=True)
    result = run_task(root, command)
    if result == 0 and doc_command:
        result = run_task(root, doc_command)
    if result:
        stamp.unlink(missing_ok=True)
        return result
    if use_cache and key_after() == key:
        cache_dir.mkdir(parents=True, exist_ok=True)
        temporary = stamp.with_suffix(f".{os.getpid()}.tmp")
        temporary.write_text(json.dumps({"schema": CACHE_SCHEMA, "key": key, "passed": True}) + "\n")
        temporary.replace(stamp)
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--suite", choices=["fast", "qualification", "full"], default="fast")
    parser.add_argument("--since", help="Select changed crates and dependents relative to this Git commit/ref")
    parser.add_argument("-p", "--package", action="append", help="Select a crate (repeatable)")
    parser.add_argument("--no-cache", action="store_true", help="Execute every selected task without reusing/writing pass results")
    parser.add_argument("--plan", action="store_true", help="Show selected crates without compiling or testing")
    parser.add_argument("--check-partition", action="store_true", help="Verify that fast + qualification cover all enabled tests, without executing them")
    args = parser.parse_args()
    start = time.monotonic()
    packages, dependencies = workspace(ROOT)
    paths = package_paths(ROOT, packages)
    selected = set(packages)
    if args.since:
        try:
            selected = affected_packages(changed_files(ROOT, args.since), paths, dependencies)
        except subprocess.CalledProcessError:
            print(f"Cannot resolve {args.since!r}; conservatively selecting the whole workspace.", file=sys.stderr)
    if args.package:
        unknown = set(args.package) - set(packages)
        if unknown:
            parser.error("unknown packages: " + ", ".join(sorted(unknown)))
        selected &= set(args.package)
    # Dependencies first makes the output easy to follow and warms their builds.
    selected = sorted(selected, key=lambda name: (len(closure({name}, dependencies)), name))
    print(f"Rust {args.suite}: " + (", ".join(selected) or "no affected packages"), flush=True)
    if args.plan or not selected:
        return 0
    nextest = ROOT / "target/test-tools/cargo-nextest"
    if not nextest.is_file():
        parser.error("Nextest is not installed. Run `bun run test:setup` once.")
    # Ignore machine-global Nextest configuration: repository config defines the task.
    version = capture([str(nextest), "--version"])
    if not version.startswith("cargo-nextest 0.9.148 "):
        parser.error("Nextest version differs from the pinned runner; run `bun run test:setup`.")
    if args.check_partition:
        check_partition(ROOT, nextest)
        return 0
    context = {
        "rustc": capture(["rustc", "-vV"]), "cargo": capture(["cargo", "--version"]),
        "nextest": version, "os": platform.platform(), "machine": platform.machine(),
        "python": sys.version,
        "environment": relevant_environment(os.environ),
    }
    if sys.platform == "darwin":
        context["clang"] = capture(["xcrun", "clang", "--version"])
        context["sdk"] = capture(["xcrun", "--show-sdk-version"])
    cargo_home = Path(os.environ.get("CARGO_HOME", str(Path.home() / ".cargo")))
    cache_dir = ROOT / "target/gitjuke-test-cache"
    for name in selected:
        def current_key():
            current_context = dict(context)
            current_context["user-cargo-config"] = {
                name: hashlib.sha256((cargo_home / name).read_bytes()).hexdigest()
                for name in ["config", "config.toml"] if (cargo_home / name).is_file()
            }
            return task_key(ROOT, name, args.suite, paths, dependencies, current_context)
        key = current_key()
        profile = "default" if args.suite == "fast" else args.suite
        command = [str(nextest), "nextest", "run", "--locked", "--offline", "-p", name,
                   *selection_args(args.suite), "--no-tests=pass"]
        # Nextest does not run doctests. Include them in fast/full task results.
        docs = ["cargo", "test", "--doc", "--locked", "--offline", "-p", name] if args.suite != "qualification" else None
        # Desktop build metadata depends on Git state, frontend files, and build
        # time. Always execute it rather than pretending these are hermetic inputs.
        use_cache = not args.no_cache and name != "gitjuke"
        report = ROOT / "target/nextest" / profile / "junit.xml"
        destination = ROOT / "target/test-reports" / args.suite / (name + ".xml")
        destination.unlink(missing_ok=True)
        report.unlink(missing_ok=True)
        result = execute(ROOT, name, args.suite, key, current_key, cache_dir, use_cache, command, docs)
        if report.is_file():
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(report, destination)
        if result:
            return result
    print(f"Rust {args.suite} passed in {time.monotonic() - start:.2f}s", flush=True)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)
    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"Rust test runner failed: {error}", file=sys.stderr)
        sys.exit(1)
