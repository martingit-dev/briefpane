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
import uuid
from pathlib import Path

from . import __version__
from .adapters import ADAPTERS, get
from .config import LAYOUTS, Config, load
from .themes import THEMES


def _view(argv: list[str]) -> None:
    p = argparse.ArgumentParser(prog="briefpane view")
    p.add_argument("agent", choices=sorted(ADAPTERS))
    p.add_argument("--cwd", type=Path, default=Path.cwd())
    p.add_argument("--file", type=Path, help="a transcript to follow instead of the newest")
    p.add_argument("--since", type=float, default=0.0, help=argparse.SUPPRESS)
    p.add_argument("--new", action="store_true", help=argparse.SUPPRESS)
    p.add_argument("--agent-pane", help=argparse.SUPPRESS)
    p.add_argument("--theme", choices=sorted(THEMES), default=load().theme)
    a = p.parse_args(argv)
    from .app import run

    run(get(a.agent), a.cwd.resolve(), a.since, a.file, a.theme, a.new, a.agent_pane)


def tmux_plan(
    layout: str,
    in_tmux: bool,
    agent_cmd: list[str],
    viewer: list[str],
    cwd: Path,
    cfg: Config,
) -> tuple[list[list[str]], list[str]]:
    """The tmux commands to run first, then the argv to exec. In bottom and side
    the agent keeps the pane you type in; in full it runs in a hidden window and
    briefpane types into it."""
    if layout == "full":
        name = f"bp-{uuid.uuid4().hex[:6]}"
        hidden = ["new-window", "-d", "-n", name, "-c", str(cwd), shlex.join(agent_cmd)]
        target = [*viewer, "--agent-pane", f":{name}" if in_tmux else f"{name}:{name}"]
        if in_tmux:
            return [["tmux", *hidden]], target
        return [], [
            "tmux",
            "new-session",
            "-s",
            name,
            "-c",
            str(cwd),
            shlex.join(target),
            ";",
            *hidden,
        ]
    if layout == "bottom":
        # -b puts briefpane above the agent; the agent keeps agent_height rows.
        split = ["split-window", "-v", "-b", "-d", "-l", _complement(cfg.agent_height)]
    else:
        split = ["split-window", "-h", "-d", "-l", cfg.pane_width]
    split += ["-c", str(cwd), shlex.join(viewer)]
    if in_tmux:
        return [["tmux", *split]], agent_cmd
    return [], ["tmux", "new-session", "-c", str(cwd), shlex.join(agent_cmd), ";", *split]


def _complement(size: str) -> str:
    """The share of the window briefpane takes once the agent has ``size``."""
    return f"{100 - int(size.rstrip('%'))}%"


def _launch(agent: str, rest: list[str], theme: str, layout: str, cfg: Config) -> None:
    adapter = get(agent)
    cwd = Path.cwd()
    since = time.time() - 1
    viewer = [sys.executable, "-m", "briefpane", "view", agent, "--cwd", str(cwd)]
    viewer += ["--since", str(since), "--theme", theme]
    args, transcript = adapter.prepare(rest, cwd)
    if transcript is not None:
        viewer += ["--file", str(transcript)]
    elif not adapter.resumes(rest):
        # Only a session that did not exist at launch can be this run's.
        viewer += ["--new"]
    agent_cmd = [adapter.command, *args]
    if not shutil.which(adapter.command):
        raise SystemExit(f"briefpane: {adapter.command!r} is not on PATH")
    in_tmux = bool(os.environ.get("TMUX"))
    if not in_tmux and not shutil.which("tmux"):
        print(f"briefpane: no tmux; run `{shlex.join(viewer)}` in another terminal.")
        os.execvp(agent_cmd[0], agent_cmd)
    before, final = tmux_plan(layout, in_tmux, agent_cmd, viewer, cwd, cfg)
    for cmd in before:
        subprocess.run(cmd, check=True)
    os.execvp(final[0], final)


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
    cfg = load()
    p.add_argument("--theme", choices=sorted(THEMES), default=cfg.theme)
    p.add_argument("--layout", choices=LAYOUTS, default=cfg.layout)
    p.add_argument("agent", choices=sorted(ADAPTERS))
    p.add_argument("args", nargs=argparse.REMAINDER, help="passed to the agent")
    a = p.parse_args(argv)
    _launch(a.agent, a.args, a.theme, a.layout, cfg)
