"""Follow a growing JSONL file: whole lines only, from where we left off."""

from __future__ import annotations

from pathlib import Path


class Tail:
    def __init__(self, path: Path):
        self.path = path
        self.offset = 0
        self.partial = ""

    def read(self) -> list[str]:
        try:
            with self.path.open("rb") as fh:
                fh.seek(self.offset)
                chunk = fh.read()
        except OSError:
            return []
        self.offset += len(chunk)
        text = self.partial + chunk.decode("utf-8", errors="replace")
        lines = text.split("\n")
        # The last piece may be a line still being written.
        self.partial = lines.pop()
        return [line for line in lines if line.strip()]
