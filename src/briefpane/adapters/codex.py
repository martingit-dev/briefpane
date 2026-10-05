"""Codex writes one JSONL per session under
``~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl``; its first record names the
cwd. Tool work runs as ``exec`` calls whose input is a script: file edits ride
in it as apply_patch markers, reads as shell commands."""

from __future__ import annotations

import json
import os
import re
from pathlib import Path

from ..model import AssistantText, Event, ToolUse, UserText, Verb
from .base import Adapter

_PATCH = re.compile(r"\*\*\* (Update|Add|Delete) File: ([^\n\\\"]+)")
_PATCH_VERB: dict[str, Verb] = {"Update": "edited", "Add": "new", "Delete": "deleted"}
_CMD = re.compile(r"cmd\s*:\s*\"((?:[^\"\\]|\\.)*)\"")
_READ = re.compile(r"(?:^|[;&|]\s*)(?:cat|head|tail|nl -ba|sed -n '[^']*')\s+((?:/|~)[^\s;&|'\"]+)")


def _skip_user(text: str) -> bool:
    t = text.lstrip()
    return not t or t.startswith(("<", "# AGENTS.md"))


class CodexAdapter(Adapter):
    name = "codex"
    command = "codex"

    def __init__(self, root: Path | None = None):
        self.root = root or Path(os.environ.get("CODEX_HOME", Path.home() / ".codex")) / "sessions"

    def transcripts(self, cwd: Path) -> list[Path]:
        if not self.root.is_dir():
            return []
        found = []
        for path in self.root.glob("*/*/*/rollout-*.jsonl"):
            try:
                with path.open() as fh:
                    first = json.loads(fh.readline() or "{}")
            except (OSError, json.JSONDecodeError):
                continue
            if (first.get("payload") or {}).get("cwd") == str(cwd):
                found.append(path)
        return found

    def parse(self, record: dict) -> list[Event]:
        if record.get("type") != "response_item":
            return []
        p = record.get("payload") or {}
        kind = p.get("type")
        if kind == "message":
            texts = [
                c.get("text", "")
                for c in p.get("content") or []
                if isinstance(c, dict) and c.get("type") in ("input_text", "output_text")
            ]
            if p.get("role") == "user":
                texts = [t.strip() for t in texts if not _skip_user(t)]
                return [UserText("\n".join(texts))] if texts else []
            if p.get("role") == "assistant":
                text = "\n".join(t for t in texts if t.strip()).strip()
                return [AssistantText(text)] if text else []
            return []
        if kind in ("custom_tool_call", "function_call"):
            name = p.get("name") or "tool"
            raw = p.get("input") or p.get("arguments") or ""
            return [ToolUse(name, _summary(name, raw), tuple(_files(raw)))]
        return []


def _commands(raw: str) -> list[str]:
    return [json.loads(f'"{m}"') for m in _CMD.findall(raw)]


def _summary(name: str, raw: str) -> str:
    if "*** Begin Patch" in raw:
        paths = [m[1] for m in _PATCH.findall(raw)]
        return "patch " + ", ".join(Path(p).name for p in paths[:3]) if paths else "patch"
    cmds = _commands(raw)
    if cmds:
        return cmds[0].splitlines()[0][:120]
    return name


def _files(raw: str) -> list[tuple[str, Verb]]:
    files: list[tuple[str, Verb]] = [
        (path.strip(), _PATCH_VERB[op]) for op, path in _PATCH.findall(raw)
    ]
    for cmd in _commands(raw):
        files += [(os.path.expanduser(path), "read") for path in _READ.findall(cmd)]
    return files
