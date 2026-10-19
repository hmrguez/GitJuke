#!/usr/bin/env python3
"""Select native CI conservatively, and validate the final required-check gate."""
import argparse
import json
import os
from pathlib import Path, PurePosixPath
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent


def native_required(paths):
    for path in paths:
        # The IPC bridge is a shared contract even though it lives beside the UI.
        if path == "src/api.ts" or path.endswith(".rs"):
            return True
        if path.startswith(("src/", "public/", "docs/", ".scratch/")):
            continue
        if "/" not in path and (path.endswith(".md") or path == "index.html"):
            continue
        if PurePosixPath(path).parent == PurePosixPath("scripts") and path.endswith((".test.tsx", ".test.mjs")):
            continue
        # Rust, Tauri, root configuration/lockfiles, CI and build/test tooling,
        # and any new/unrecognized path all keep native validation enabled.
        return True
    return False


def git(root, *args):
    return subprocess.check_output(["git", *args], cwd=root, stderr=subprocess.PIPE)


def changed_paths(root, event_name, event):
    if event_name == "pull_request":
        base = event["pull_request"]["base"]["sha"]
        head = event["pull_request"]["head"]["sha"]
    elif event_name == "push":
        base, head = event["before"], event["after"]
    else:
        raise ValueError("event has no supported comparison range")
    for revision in [base, head]:
        if not isinstance(revision, str) or not re.fullmatch(r"[0-9a-f]{40}|[0-9a-f]{64}", revision) or set(revision) == {"0"}:
            raise ValueError("comparison commit is missing or invalid")
    if event_name == "pull_request":
        base = git(root, "merge-base", base, head).decode().strip()
    # Both sides of renames are checked: moving a Rust file into a frontend
    # directory must still run native validation for the removed Rust path.
    output = git(root, "diff", "--name-only", "--no-renames", "-z", base, head, "--")
    return [os.fsdecode(path) for path in output.split(b"\0") if path]


def detect(root, event_name, event, ref="", force_full=False):
    if force_full or ref.startswith("refs/tags/"):
        return True, "release or reusable workflow requires full native validation"
    try:
        paths = changed_paths(root, event_name, event)
    except (KeyError, TypeError, ValueError, OSError, subprocess.CalledProcessError):
        return True, "comparison unavailable; selecting full native validation"
    required = native_required(paths)
    reason = "native, shared, or unclassified inputs changed" if required else "only frontend/documentation inputs changed"
    return required, reason


def gate_passes(changes, frontend, native, rust, desktop):
    if changes != "success" or frontend != "success":
        return False
    expected = {"true": "success", "false": "skipped"}.get(native)
    return expected is not None and rust == expected and desktop == expected


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["detect", "gate"])
    args = parser.parse_args()
    if args.command == "gate":
        results = {name: os.environ.get(name, "") for name in [
            "CHANGES_RESULT", "FRONTEND_RESULT", "NATIVE_REQUIRED", "RUST_RESULT", "DESKTOP_RESULT",
        ]}
        if not gate_passes(*results.values()):
            print("CI gate failed: " + json.dumps(results, sort_keys=True), file=sys.stderr)
            return 1
        print("All required checks passed; only intentionally omitted native jobs may be skipped.")
        return 0
    try:
        event = json.loads(Path(os.environ["GITHUB_EVENT_PATH"]).read_text())
    except (KeyError, OSError, ValueError):
        event = {}
    required, reason = detect(
        ROOT, os.environ.get("GITHUB_EVENT_NAME", ""), event,
        os.environ.get("GITHUB_REF", ""), os.environ.get("CI_FORCE_FULL") == "true",
    )
    value = str(required).lower()
    print(f"Native validation required: {value} ({reason})")
    with open(os.environ["GITHUB_OUTPUT"], "a") as output:
        output.write(f"native_required={value}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
