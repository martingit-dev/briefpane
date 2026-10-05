"""``briefpane <agent> [agent args]`` starts the agent with the pane beside it
in tmux. ``briefpane view <agent>`` opens just the pane, on the newest session
in this directory (or ``--file``)."""

from __future__ import annotations

import argparse
import os
import shlex
import shutil
import subprocess
import sys
import time
from pathlib import Path

from . import __version__
from .adapters import ADAPTERS, get
from .themes import THEMES

_WIDTH = "40%"


def _view(argv: list[str]) -> None:
    p = argparse.ArgumentParser(prog="briefpane view")
    p.add_argument("agent", choices=sorted(ADAPTERS))
    p.add_argument("--cwd", type=Path, default=Path.cwd())
    p.add_argument("--file", type=Path, help="a transcript to follow instead of the newest")
    p.add_argument("--since", type=float, default=0.0, help=argparse.SUPPRESS)
    p.add_argument("--theme", choices=sorted(THEMES), default="matrix")
    a = p.parse_args(argv)
    from .app import run

    run(get(a.agent), a.cwd.resolve(), a.since, a.file, a.theme)


def _launch(agent: str, rest: list[str], theme: str) -> None:
    adapter = get(agent)
    cwd = Path.cwd()
    since = time.time() - 1
    viewer = [sys.executable, "-m", "briefpane", "view", agent, "--cwd", str(cwd)]
    viewer += ["--since", str(since), "--theme", theme]
    agent_cmd = [adapter.command, *rest]
    if not shutil.which(adapter.command):
        raise SystemExit(f"briefpane: {adapter.command!r} is not on PATH")
    if os.environ.get("TMUX"):
        split = ["tmux", "split-window", "-h", "-d", "-l", _WIDTH, "-c", str(cwd)]
        subprocess.run([*split, shlex.join(viewer)], check=True)
        os.execvp(agent_cmd[0], agent_cmd)
    if not shutil.which("tmux"):
        print(f"briefpane: no tmux; run `{shlex.join(viewer)}` in another terminal.")
        os.execvp(agent_cmd[0], agent_cmd)
    os.execvp(
        "tmux",
        [
            "tmux",
            "new-session",
            "-c",
            str(cwd),
            shlex.join(agent_cmd),
            ";",
            "split-window",
            "-h",
            "-d",
            "-l",
            _WIDTH,
            "-c",
            str(cwd),
            shlex.join(viewer),
        ],
    )


def main(argv: list[str] | None = None) -> None:
    argv = list(sys.argv[1:] if argv is None else argv)
    if argv and argv[0] == "view":
        _view(argv[1:])
        return
    p = argparse.ArgumentParser(
        prog="briefpane",
        description="Run a coding agent with a pane that keeps the brief.",
        epilog="briefpane view <agent> opens only the pane.",
    )
    p.add_argument("--version", action="version", version=f"briefpane {__version__}")
    p.add_argument("--theme", choices=sorted(THEMES), default="matrix")
    p.add_argument("agent", choices=sorted(ADAPTERS))
    p.add_argument("args", nargs=argparse.REMAINDER, help="passed to the agent")
    a = p.parse_args(argv)
    _launch(a.agent, a.args, a.theme)
