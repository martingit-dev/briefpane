"""The pane: band on top, the conversation on the left, files on the right."""

from __future__ import annotations

import asyncio
from pathlib import Path
from typing import ClassVar

from textual.app import App, ComposeResult
from textual.containers import Horizontal, VerticalScroll
from textual.widgets import Footer, Static

from . import view
from .adapters import Adapter
from .model import Session
from .tail import Tail
from .themes import ORDER, THEMES

_POLL_S = 0.4


class BriefPane(App):
    CSS = """
    #band { height: auto; padding: 0 1; background: $panel; color: $foreground; }
    #main { height: 1fr; }
    #convo { width: 1fr; padding: 1 2 0 1; }
    #files { width: 42; padding: 1 1; border-left: solid $panel; }
    #files.hidden { display: none; }
    .turn { margin-bottom: 0; }
    """
    BINDINGS: ClassVar = [
        ("t", "cycle_theme", "theme"),
        ("f", "toggle_files", "files"),
        ("end", "scroll_end", "latest"),
        ("q", "quit", "quit"),
    ]

    def __init__(
        self,
        adapter: Adapter,
        cwd: Path,
        since: float = 0.0,
        path: Path | None = None,
        theme: str = "matrix",
    ):
        super().__init__()
        self.adapter = adapter
        self.cwd = cwd
        self.since = since
        self.path = path
        self.start_theme = theme
        self.session = Session()
        self.rendered = 0

    def compose(self) -> ComposeResult:
        yield Static(id="band")
        with Horizontal(id="main"):
            yield VerticalScroll(id="convo")
            yield Static(id="files")
        yield Footer()

    def on_mount(self) -> None:
        for theme in THEMES.values():
            self.register_theme(theme)
        self.theme = THEMES[self.start_theme].name
        self.title = f"briefpane · {self.adapter.name}"
        self.query_one("#band", Static).update(
            view.Text(f"waiting for a {self.adapter.name} session in {self.cwd}", style="dim")
        )
        self.run_worker(self.follow(), exclusive=True)

    async def follow(self) -> None:
        while self.path is None:
            self.path = self.adapter.find(self.cwd, self.since)
            if self.path is None:
                await asyncio.sleep(_POLL_S)
        tail = Tail(self.path)
        while True:
            lines = tail.read()
            if lines:
                for line in lines:
                    for event in self.adapter.parse_line(line):
                        self.session.add(event)
                await self.redraw()
            await asyncio.sleep(_POLL_S)

    async def redraw(self) -> None:
        convo = self.query_one("#convo", VerticalScroll)
        at_end = convo.scroll_offset.y >= convo.max_scroll_y - 2
        widgets = list(convo.query(".turn"))
        # Earlier turns are final; only the last one rendered can still grow.
        start = max(0, self.rendered - 1)
        for i in range(start, len(self.session.turns)):
            body = view.turn(self.session.turns[i])
            if i < len(widgets):
                widgets[i].update(body)
            else:
                await convo.mount(Static(body, classes="turn"))
        self.rendered = len(self.session.turns)
        width = self.size.width or 200
        self.query_one("#band", Static).update(view.band(self.session, width))
        rows = self.size.height or 40
        self.files_view = view.files(self.session, self.cwd, rows)
        self.query_one("#files", Static).update(self.files_view)
        if at_end:
            convo.scroll_end(animate=False)

    def action_cycle_theme(self) -> None:
        names = [THEMES[n].name for n in ORDER]
        current = names.index(self.theme) if self.theme in names else -1
        self.theme = names[(current + 1) % len(names)]

    def action_toggle_files(self) -> None:
        self.query_one("#files").toggle_class("hidden")

    def action_scroll_end(self) -> None:
        self.query_one("#convo", VerticalScroll).scroll_end(animate=False)


def run(adapter: Adapter, cwd: Path, since: float, path: Path | None, theme: str) -> None:
    BriefPane(adapter, cwd, since, path, theme).run()
