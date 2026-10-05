"""The adapter contract: one per agent, so briefpane knows how to start the
agent, where it writes its transcript, and what each record means."""

from __future__ import annotations

import json
from abc import ABC, abstractmethod
from pathlib import Path

from ..model import Event


class Adapter(ABC):
    name: str
    command: str

    @abstractmethod
    def transcripts(self, cwd: Path) -> list[Path]:
        """Transcript files of sessions run in ``cwd``, any order."""

    @abstractmethod
    def parse(self, record: dict) -> list[Event]:
        """The events one transcript record carries; [] for bookkeeping."""

    def find(self, cwd: Path, since: float = 0.0) -> Path | None:
        """The newest transcript for ``cwd`` written at or after ``since``."""
        fresh = [p for p in self.transcripts(cwd) if p.stat().st_mtime >= since]
        return max(fresh, key=lambda p: p.stat().st_mtime, default=None)

    def parse_line(self, line: str) -> list[Event]:
        try:
            record = json.loads(line)
        except json.JSONDecodeError:
            return []
        return self.parse(record) if isinstance(record, dict) else []
