"""Adapters by agent name. A new agent is one Adapter subclass added here."""

from .base import Adapter
from .claude import ClaudeAdapter
from .codex import CodexAdapter

ADAPTERS: dict[str, type[Adapter]] = {"claude": ClaudeAdapter, "codex": CodexAdapter}


def get(name: str) -> Adapter:
    try:
        return ADAPTERS[name]()
    except KeyError:
        raise SystemExit(f"briefpane: no adapter for {name!r}; known: {', '.join(ADAPTERS)}")


__all__ = ["ADAPTERS", "Adapter", "get"]
