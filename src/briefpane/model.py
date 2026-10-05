"""What a transcript becomes, independent of which agent wrote it.

An adapter turns its agent's records into Events; Session folds Events into
turns, a reply per turn, and the files the agent touched.
"""

from __future__ import annotations

import re
from collections import Counter
from dataclasses import dataclass, field
from typing import Literal

Verb = Literal["read", "edited", "new", "deleted"]


@dataclass(frozen=True)
class UserText:
    text: str


@dataclass(frozen=True)
class AssistantText:
    text: str


@dataclass(frozen=True)
class ToolUse:
    name: str
    summary: str
    files: tuple[tuple[str, Verb], ...] = ()


Event = UserText | AssistantText | ToolUse

LABELS = ("answer", "why", "proof", "you", "more")
_LABEL = re.compile(r"^\s*(?:\*\*)?(answer|why|proof|you|more)(?:\*\*)?\s*:\s*(.*)$", re.IGNORECASE)


@dataclass
class Reply:
    """A reply as labelled lines (answer, why, proof, you, more) when the agent
    wrote them, else its first line as the answer and the rest as body."""

    labels: dict[str, str] = field(default_factory=dict)
    body: str = ""

    @property
    def answer(self) -> str:
        return self.labels.get("answer", "")

    @property
    def you(self) -> str:
        return self.labels.get("you", "")

    @property
    def is_structured(self) -> bool:
        return bool(self.labels)


def parse_reply(text: str) -> Reply:
    labels: dict[str, str] = {}
    rest: list[str] = []
    current: str | None = None
    for line in text.strip().strip("`").splitlines():
        m = _LABEL.match(line)
        if m:
            current = m.group(1).lower()
            labels[current] = m.group(2).replace("**", "").strip()
        elif current and line.strip() and line.startswith(" "):
            # A wrapped continuation belongs to the label above it.
            labels[current] += " " + line.strip()
        else:
            current = None
            rest.append(line)
    body = "\n".join(rest).strip()
    if labels:
        return Reply(labels=labels, body=body)
    first, _, more = body.partition("\n")
    return Reply(labels={"answer": first.strip()} if first.strip() else {}, body=more.strip())


@dataclass
class Turn:
    prompt: str
    tools: list[ToolUse] = field(default_factory=list)
    notes: list[str] = field(default_factory=list)
    reply_text: str = ""

    @property
    def reply(self) -> Reply:
        return parse_reply(self.reply_text)

    def tool_summary(self) -> str:
        if not self.tools:
            return ""
        counts = Counter(t.name for t in self.tools)
        parts = [f"{name} x{n}" if n > 1 else name for name, n in counts.most_common()]
        return f"ran {len(self.tools)} tool{'s' if len(self.tools) > 1 else ''}: " + ", ".join(
            parts
        )


_RANK = {"read": 0, "edited": 1, "deleted": 2, "new": 3}


@dataclass
class FileTouch:
    path: str
    verb: Verb
    turn: int


@dataclass
class Session:
    turns: list[Turn] = field(default_factory=list)
    files: dict[str, FileTouch] = field(default_factory=dict)

    def add(self, event: Event) -> None:
        if isinstance(event, UserText):
            self.turns.append(Turn(prompt=event.text))
            return
        if not self.turns:
            self.turns.append(Turn(prompt=""))
        turn = self.turns[-1]
        if isinstance(event, ToolUse):
            # Text before a tool call was narration; the reply is what comes after.
            if turn.reply_text:
                turn.notes.append(turn.reply_text)
                turn.reply_text = ""
            turn.tools.append(event)
            for path, verb in event.files:
                self.touch(path, verb)
        else:
            turn.reply_text = (
                f"{turn.reply_text}\n\n{event.text}" if turn.reply_text else event.text
            )

    def touch(self, path: str, verb: Verb) -> None:
        prior = self.files.get(path)
        # A stronger verb (an edit over a read, a new file over an edit) stays.
        if prior and _RANK[prior.verb] > _RANK[verb]:
            verb = prior.verb
        self.files.pop(path, None)
        self.files[path] = FileTouch(path, verb, len(self.turns))

    @property
    def goal(self) -> str:
        return next((t.prompt for t in self.turns if t.prompt), "")

    @property
    def latest_reply(self) -> Reply:
        for turn in reversed(self.turns):
            if turn.reply_text:
                return turn.reply
        return Reply()

    def files_this_turn(self) -> list[FileTouch]:
        n = len(self.turns)
        return [f for f in reversed(self.files.values()) if f.turn == n]

    def files_changed(self) -> list[FileTouch]:
        return [f for f in reversed(self.files.values()) if f.verb != "read"]
