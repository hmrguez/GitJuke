#!/usr/bin/env python3
"""Clear repository build output while keeping the installed Nextest runner."""
from pathlib import Path
import shutil


def remove(path):
    if path.is_symlink() or path.is_file():
        path.unlink()
    elif path.exists():
        shutil.rmtree(path)


def clean(root):
    target = root / "target"
    # Never follow a redirected build directory outside the repository.
    if target.is_symlink():
        target.unlink()
    elif target.is_dir():
        for path in target.iterdir():
            if path.name != "test-tools":
                remove(path)
    for relative in ("dist", "node_modules/.vite", ".vite", ".scratch/gitjuke/prototype/dist"):
        remove(root / relative)


if __name__ == "__main__":
    clean(Path(__file__).resolve().parent.parent)
    print("Cleared build output and test caches; kept dependencies and Nextest.")
