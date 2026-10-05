"""How a session reads on screen: the band, one turn, the files pane. Pure
functions over the model and a palette, so they are testable without a
terminal."""

from __future__ import annotations

import re
from dataclasses import dataclass
from pathlib import Path

from rich.console import Group
from rich.table import Table
from rich.text import Text

from .model import FileTouch, Session, Turn

_LABEL_WIDTH = 7
_CODE = re.compile(r"`([^`\n]+)`")
_PULL = re.compile(r"(?:^|\s*·\s*)(\d+)\s+")


@dataclass(frozen=True)
class Palette:
    """Four levels of emphasis plus code, so hierarchy survives a one-hue theme."""

    text: str
    dim: str
    strong: str
    prompt: str
    code: str


def short(path: str, cwd: Path) -> str:
    p = Path(path)
    try:
        return str(p.relative_to(cwd))
    except ValueError:
        pass
    home = str(Path.home())
    text = path.replace(home, "~", 1) if path.startswith(home) else path
    parts = text.split("/")
    return "…/" + "/".join(parts[-3:]) if len(parts) > 4 else text


def clip(text: str, n: int) -> str:
    text = " ".join(text.split())
    return text if len(text) <= n else text[: n - 1] + "…"


def inline(text: str, style: str, pal: Palette) -> Text:
    """``text`` with `code` spans shown in the code colour, backticks dropped."""
    out = Text(style=style)
    last = 0
    for m in _CODE.finditer(text):
        out.append(text[last : m.start()])
        out.append(m.group(1), style=pal.code)
        last = m.end()
    out.append(text[last:])
    return out


def pulls(text: str) -> list[tuple[str, str]]:
    """``1 a thing · 2 another`` as [("1", "a thing"), ("2", "another")]."""
    marks = list(_PULL.finditer(text))
    if not marks:
        return [("", text.strip())] if text.strip() else []
    return [
        (m.group(1), text[m.end() : marks[i + 1].start() if i + 1 < len(marks) else None].strip())
        for i, m in enumerate(marks)
    ]


def band(session: Session, pal: Palette, width: int = 200) -> Text:
    reply = session.latest_reply
    room = max(20, (width - 24) // 3)
    out = Text()
    for label, value in (("goal", session.goal), ("now", reply.answer), ("you", reply.you)):
        if value:
            out.append(f"{label} ", style=pal.dim)
            plain = _CODE.sub(r"\1", value)
            out.append(clip(plain, room), style=pal.strong if label == "you" else pal.text)
            out.append("   ")
    return out if out.plain else Text("waiting for the first turn", style=pal.dim)


def turn(t: Turn, n: int, pal: Palette) -> Group:
    parts: list = [Text(f"── {n} " + "─" * 200, style=pal.dim, no_wrap=True, overflow="crop")]
    if t.prompt:
        prompt = Text("▌ ", style=pal.prompt)
        prompt.append_text(inline(clip(t.prompt, 600), f"bold {pal.prompt}", pal))
        parts.append(prompt)
    if t.tools:
        parts.append(Text("  " + t.tool_summary(), style=f"italic {pal.dim}"))
    reply = t.reply
    if reply.is_structured and (set(reply.labels) - {"answer"} or not reply.body):
        grid = Table.grid(padding=(0, 1))
        grid.add_column(width=_LABEL_WIDTH, style=pal.dim)
        grid.add_column(ratio=1)
        for label in ("answer", "why", "proof", "you"):
            if label in reply.labels:
                style = f"bold {pal.strong}" if label in ("answer", "you") else pal.text
                grid.add_row(label, inline(reply.labels[label], style, pal))
        for i, (num, item) in enumerate(pulls(reply.labels.get("more", ""))):
            row = Text(f"{num:>2}  " if num else "    ", style=pal.strong)
            row.append_text(inline(item, pal.dim, pal))
            grid.add_row("more" if i == 0 else "", row)
        parts.append(grid)
        if reply.body:
            parts.append(inline(reply.body, pal.text, pal))
    elif reply.answer:
        parts.append(inline(reply.answer, f"bold {pal.strong}", pal))
        if reply.body:
            parts.append(inline(reply.body, pal.text, pal))
    elif t.tools:
        parts.append(Text("  working…", style=pal.dim))
    parts.append(Text(""))
    return Group(*parts)


def files(session: Session, cwd: Path, pal: Palette, rows: int = 40) -> Group:
    def section(title: str, items: list[FileTouch], limit: int) -> list:
        out: list = [Text(title.upper(), style=pal.dim)]
        if not items:
            out.append(Text("nothing yet", style=pal.dim))
        for f in items[:limit]:
            path = short(f.path, cwd)
            folder, _, name = path.rpartition("/")
            line = Text(f"{f.verb:<7} ", style=pal.dim)
            line.append(name, style=pal.strong)
            if folder:
                line.append(f"  {folder}", style=pal.dim)
            line.no_wrap, line.overflow = True, "ellipsis"
            out.append(line)
        return out

    half = max(3, rows // 2 - 2)
    return Group(
        *section("this turn", session.files_this_turn(), half),
        Text(""),
        *section("changed this session", session.files_changed(), half),
    )
