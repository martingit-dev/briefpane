"""The agent runs hidden in a tmux pane; briefpane types into it and reads its
screen only to notice when it is waiting on a menu or an approval."""

from __future__ import annotations

import asyncio
import re

# The footer an agent's menu or approval dialog ends with. A reply's own words
# ("Do you want to...") must never count, so only these hints do.
_MENU = re.compile(r"Esc to (?:cancel|exit|go back)|Enter to (?:confirm|select|set)", re.IGNORECASE)


async def _tmux(*args: str) -> tuple[int, str]:
    proc = await asyncio.create_subprocess_exec(
        "tmux", *args, stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.DEVNULL
    )
    out, _ = await proc.communicate()
    return proc.returncode or 0, out.decode(errors="replace")


class AgentPane:
    def __init__(self, target: str):
        self.target = target

    async def send(self, text: str) -> None:
        """Type ``text`` and submit it. Enter goes separately: sent with the
        text, a TUI reads it as part of a paste and does not submit."""
        if text:
            await _tmux("send-keys", "-t", self.target, "-l", text)
            await asyncio.sleep(0.15)
        await _tmux("send-keys", "-t", self.target, "Enter")

    async def key(self, name: str) -> None:
        await _tmux("send-keys", "-t", self.target, name)

    async def screen(self) -> str:
        code, out = await _tmux("capture-pane", "-p", "-t", self.target)
        return out if code == 0 else ""

    async def alive(self) -> bool:
        code, out = await _tmux("display-message", "-p", "-t", self.target, "#{pane_dead}")
        return code == 0 and out.strip() != "1"

    async def started(self, timeout: float = 8.0) -> bool:
        """Wait for the agent's pane to exist; briefpane can start first."""
        loop = asyncio.get_running_loop()
        end = loop.time() + timeout
        while loop.time() < end:
            if await self.alive():
                return True
            await asyncio.sleep(0.2)
        return False

    async def close(self) -> None:
        await _tmux("kill-pane", "-t", self.target)

    async def show(self) -> None:
        await _tmux("select-window", "-t", self.target)


def waiting_menu(screen: str) -> str | None:
    """The agent's menu, as its last lines, when its screen shows one."""
    lines = [line.rstrip() for line in screen.splitlines() if line.strip()]
    if not any(_MENU.search(line) for line in lines[-3:]):
        return None
    return "\n".join(lines[-14:])
