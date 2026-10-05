"""Claude Code writes one JSONL per session under
``~/.claude/projects/<cwd with every non-alphanumeric as ->/<session>.jsonl``."""

from __future__ import annotations

import os
import re
from pathlib import Path

from ..model import AssistantText, Event, ToolUse, UserText
from .base import Adapter

_VERBS = {"Read": "read", "Edit": "edited", "MultiEdit": "edited", "NotebookEdit": "edited"}


def project_dir(cwd: Path, root: Path | None = None) -> Path:
    root = root or Path(os.environ.get("CLAUDE_CONFIG_DIR", Path.home() / ".claude")) / "projects"
    return root / re.sub(r"[^A-Za-z0-9]", "-", str(cwd))


def _is_noise(text: str) -> bool:
    # Slash-command echoes, caveats and system reminders, not the person's words.
    return not text.strip() or text.lstrip().startswith(("<", "[Request interrupted"))


def _summary(name: str, args: dict) -> str:
    for key in ("description", "command", "file_path", "pattern", "url", "query", "prompt"):
        value = args.get(key)
        if isinstance(value, str) and value.strip():
            return value.strip().splitlines()[0][:120]
    return name


class ClaudeAdapter(Adapter):
    name = "claude"
    command = "claude"

    def __init__(self, root: Path | None = None):
        self.root = root

    def transcripts(self, cwd: Path) -> list[Path]:
        folder = project_dir(cwd, self.root)
        return list(folder.glob("*.jsonl")) if folder.is_dir() else []

    def parse(self, record: dict) -> list[Event]:
        if record.get("isMeta") or record.get("isSidechain"):
            return []
        kind = record.get("type")
        content = (record.get("message") or {}).get("content")
        if kind == "user":
            if isinstance(content, str):
                return [] if _is_noise(content) else [UserText(content.strip())]
            texts = [
                b.get("text", "")
                for b in content or []
                if isinstance(b, dict) and b.get("type") == "text"
            ]
            texts = [t.strip() for t in texts if not _is_noise(t)]
            return [UserText("\n".join(texts))] if texts else []
        if kind != "assistant" or not isinstance(content, list):
            return []
        events: list[Event] = []
        for block in content:
            if not isinstance(block, dict):
                continue
            if block.get("type") == "text" and block.get("text", "").strip():
                events.append(AssistantText(block["text"].strip()))
            elif block.get("type") == "tool_use":
                name = block.get("name", "tool")
                args = block.get("input") or {}
                files = ()
                path = args.get("file_path") or args.get("notebook_path")
                if isinstance(path, str):
                    verb = _VERBS.get(name, "edited" if name == "Write" else None)
                    if verb:
                        files = ((path, verb),)
                events.append(ToolUse(name, _summary(name, args), files))
        return events
