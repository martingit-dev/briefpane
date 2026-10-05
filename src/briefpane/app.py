"""The pane: band on top, the conversation on the left, files on the right."""

from __future__ import annotations

import asyncio
from pathlib import Path
from typing import ClassVar

from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal, VerticalScroll
from textual.widgets import Footer, Input, Static

from . import view
from .adapters import Adapter
from .agentpane import AgentPane, waiting_menu
from .model import Session
from .tail import Tail
from .themes import ORDER, PALETTES, THEMES

_POLL_S = 0.4
# Below this width the files pane would squeeze the conversation; `f` still shows it.
_FILES_MIN_WIDTH = 110


class BriefPane(App):
    CSS = """
    #band { height: auto; padding: 0 1; background: $panel; color: $foreground; }
    #main { height: 1fr; }
    #convo { width: 1fr; padding: 1 2 0 1; scrollbar-size-vertical: 1;
             scrollbar-color: $panel; scrollbar-background: $background; }
    #files { width: 42; padding: 1 1; border-left: solid $panel; }
    #files.hidden { display: none; }
    #menu { height: auto; max-height: 16; padding: 0 1; margin: 0 1; border: round $accent; }
    #menu.hidden { display: none; }
    #prompt { margin: 0 1; border: tall $panel; }
    #prompt:focus { border: tall $primary; }
    .turn { margin-bottom: 0; }
    """
    # Ctrl keys, since plain letters belong to the input box; priority so the
    # input's own editing keys do not swallow them.
    BINDINGS: ClassVar = [
        Binding("escape", "interrupt", "stop / cancel", priority=True),
        Binding("ctrl+o", "show_agent", "agent screen", priority=True),
        Binding("ctrl+t", "cycle_theme", "theme", priority=True),
        Binding("ctrl+f", "toggle_files", "files", priority=True),
        Binding("end", "scroll_end", "latest"),
        Binding("ctrl+q", "quit", "quit", priority=True),
    ]

    def __init__(
        self,
        adapter: Adapter,
        cwd: Path,
        since: float = 0.0,
        path: Path | None = None,
        theme: str = "matrix",
        new_only: bool = False,
        agent: str | None = None,
    ):
        super().__init__()
        self.adapter = adapter
        self.cwd = cwd
        self.since = since
        self.path = path
        self.start_theme = theme
        self.new_only = new_only
        self.agent = AgentPane(agent) if agent else None
        self.menu_open = False
        self.session = Session()
        self.rendered = 0
        # One redraw at a time: two interleaved ones would each mount the new turns.
        self.drawing = asyncio.Lock()

    def compose(self) -> ComposeResult:
        yield Static(id="band")
        with Horizontal(id="main"):
            yield VerticalScroll(id="convo")
            yield Static(id="files")
        if self.agent:
            yield Static(id="menu", classes="hidden")
            yield Input(id="prompt", placeholder="message the agent · @file and /commands work")
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
        if self.agent:
            self.run_worker(self.watch_agent())
            self.query_one("#prompt", Input).focus()

    async def follow(self) -> None:
        existing = frozenset(self.adapter.transcripts(self.cwd)) if self.new_only else frozenset()
        while self.path is None:
            self.path = self.adapter.find(self.cwd, self.since, existing)
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

    async def watch_agent(self) -> None:
        """Surface the agent's menus here, and close when the agent exits."""
        await self.agent.started()
        while await self.agent.alive():
            menu = waiting_menu(await self.agent.screen())
            box = self.query_one("#menu", Static)
            self.menu_open = menu is not None
            box.set_class(menu is None, "hidden")
            if menu:
                box.update(view.Text(menu))
            prompt = self.query_one("#prompt", Input)
            prompt.placeholder = (
                "the agent is asking: type an option number, or Enter to accept"
                if menu
                else "message the agent · @file and /commands work"
            )
            await asyncio.sleep(0.7)
        self.exit()

    async def on_input_submitted(self, event: Input.Submitted) -> None:
        text = event.value
        event.input.value = ""
        if self.menu_open and (text.isdigit() or not text):
            # A menu takes the keypress itself, not a typed message.
            await self.agent.key(text or "Enter")
        else:
            await self.agent.send(text)
        self.action_scroll_end()

    async def action_quit(self) -> None:
        # briefpane is the agent's only window: quitting it ends the agent too.
        if self.agent:
            await self.agent.close()
        self.exit()

    async def action_interrupt(self) -> None:
        if self.agent:
            await self.agent.key("Escape")

    async def action_show_agent(self) -> None:
        if self.agent:
            await self.agent.show()

    async def redraw(self) -> None:
        async with self.drawing:
            await self._redraw()

    async def _redraw(self) -> None:
        convo = self.query_one("#convo", VerticalScroll)
        at_end = convo.scroll_offset.y >= convo.max_scroll_y - 2
        widgets = list(convo.query(".turn"))
        # Earlier turns are final; only the last one rendered can still grow.
        start = max(0, self.rendered - 1)
        for i in range(start, len(self.session.turns)):
            body = view.turn(self.session.turns[i], i + 1, self.palette)
            if i < len(widgets):
                widgets[i].update(body)
            else:
                await convo.mount(Static(body, classes="turn"))
        self.rendered = len(self.session.turns)
        width = self.size.width or 200
        self.query_one("#band", Static).update(view.band(self.session, self.palette, width))
        rows = self.size.height or 40
        self.files_view = view.files(self.session, self.cwd, self.palette, rows)
        self.query_one("#files", Static).update(self.files_view)
        if at_end:
            convo.scroll_end(animate=False)

    @property
    def palette(self) -> view.Palette:
        return PALETTES.get(self.theme, PALETTES[THEMES["matrix"].name])

    def watch_theme(self) -> None:
        # Colours are baked into each turn; a new theme redraws them all.
        self.rendered = 0
        if self.is_mounted:
            self.call_after_refresh(self.run_worker, self.redraw())

    def on_resize(self, event) -> None:
        self.query_one("#files").set_class(event.size.width < _FILES_MIN_WIDTH, "hidden")

    def action_cycle_theme(self) -> None:
        names = [THEMES[n].name for n in ORDER]
        current = names.index(self.theme) if self.theme in names else -1
        self.theme = names[(current + 1) % len(names)]

    def action_toggle_files(self) -> None:
        self.query_one("#files").toggle_class("hidden")

    def action_scroll_end(self) -> None:
        self.query_one("#convo", VerticalScroll).scroll_end(animate=False)


def run(
    adapter: Adapter,
    cwd: Path,
    since: float,
    path: Path | None,
    theme: str,
    new_only: bool,
    agent: str | None = None,
) -> None:
    BriefPane(adapter, cwd, since, path, theme, new_only, agent).run()
