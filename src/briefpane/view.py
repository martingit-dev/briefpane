"""How a session reads on screen: the band, one turn, the files pane. Pure
functions over the model, so they are testable without a terminal."""

from __future__ import annotations

from pathlib import Path

from rich.console import Group
from rich.table import Table
from rich.text import Text

from .model import FileTouch, Session, Turn

_LABEL_WIDTH = 7


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


def band(session: Session, width: int = 200) -> Text:
    reply = session.latest_reply
    room = max(20, (width - 24) // 3)
    out = Text()
    for label, value in (("goal", session.goal), ("now", reply.answer), ("you", reply.you)):
        if value:
            out.append(f"{label} ", style="dim")
            out.append(clip(value, room), style="bold" if label == "now" else "")
            out.append("   ")
    return out if out.plain else Text("waiting for the first turn", style="dim")


def turn(t: Turn) -> Group:
    parts: list = []
    if t.prompt:
        parts.append(Text("> " + clip(t.prompt, 400), style="dim"))
    if t.tools:
        parts.append(Text("· " + t.tool_summary(), style="dim italic"))
    reply = t.reply
    if (
        reply.is_structured
        and any(k != "answer" for k in reply.labels)
        or (reply.is_structured and not reply.body)
    ):
        grid = Table.grid(padding=(0, 1))
        grid.add_column(width=_LABEL_WIDTH, style="dim")
        grid.add_column(ratio=1)
        for label in ("answer", "why", "proof", "you", "more"):
            if label in reply.labels:
                style = "bold" if label in ("answer", "you") else ("dim" if label == "more" else "")
                grid.add_row(label, Text(reply.labels[label], style=style))
        parts.append(grid)
        if reply.body:
            parts.append(Text(reply.body))
    elif reply.answer:
        parts.append(Text(reply.answer, style="bold"))
        if reply.body:
            parts.append(Text(reply.body))
    elif t.tools:
        parts.append(Text("working…", style="dim"))
    parts.append(Text(""))
    return Group(*parts)


def files(session: Session, cwd: Path, rows: int = 40) -> Group:
    def section(title: str, items: list[FileTouch], limit: int) -> list:
        out: list = [Text(title.upper(), style="dim")]
        if not items:
            out.append(Text("nothing yet", style="dim"))
        for f in items[:limit]:
            line = Text(f"{f.verb:<7} ", style="dim")
            line.append(short(f.path, cwd))
            out.append(line)
        return out

    half = max(3, rows // 2 - 2)
    return Group(
        *section("this turn", session.files_this_turn(), half),
        Text(""),
        *section("changed this session", session.files_changed(), half),
    )
