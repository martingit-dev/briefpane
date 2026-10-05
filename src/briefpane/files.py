"""File names for `@` completion: the repo's tracked files, or a shallow walk."""

from __future__ import annotations

import os
import subprocess
from pathlib import Path

_SKIP = {".git", "node_modules", ".venv", "__pycache__", "dist", "build", ".next"}
_LIMIT = 20000


def list_files(cwd: Path) -> list[str]:
    try:
        out = subprocess.run(
            ["git", "ls-files", "--cached", "--others", "--exclude-standard"],
            cwd=cwd,
            capture_output=True,
            text=True,
            timeout=3,
            check=True,
        ).stdout
        files = out.splitlines()
        if files:
            return files[:_LIMIT]
    except (OSError, subprocess.SubprocessError):
        pass
    found: list[str] = []
    for root, dirs, names in os.walk(cwd):
        dirs[:] = [d for d in dirs if d not in _SKIP and not d.startswith(".")]
        if Path(root).relative_to(cwd).parts.__len__() >= 3:
            dirs[:] = []
        found += [str(Path(root, n).relative_to(cwd)) for n in names]
        if len(found) >= _LIMIT:
            break
    return found


def matches(files: list[str], query: str, n: int = 8) -> list[str]:
    """Files whose name starts with ``query``, then paths containing it."""
    q = query.lower()
    by_name = [f for f in files if Path(f).name.lower().startswith(q)]
    by_path = [f for f in files if q in f.lower() and f not in by_name]
    return (sorted(by_name, key=len) + sorted(by_path, key=len))[:n]
