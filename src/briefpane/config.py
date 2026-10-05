"""``~/.config/briefpane/config.toml``: defaults a flag can still override."""

from __future__ import annotations

import os
import tomllib
from dataclasses import dataclass, fields
from pathlib import Path

LAYOUTS = ("bottom", "side")


@dataclass
class Config:
    theme: str = "matrix"
    # bottom: briefpane fills the window, the agent is a strip below where you
    # type. side: the agent on the left, briefpane on the right.
    layout: str = "bottom"
    agent_height: str = "30%"
    pane_width: str = "40%"


def path() -> Path:
    base = os.environ.get("XDG_CONFIG_HOME") or str(Path.home() / ".config")
    return Path(base) / "briefpane" / "config.toml"


def load(file: Path | None = None) -> Config:
    file = file or path()
    if not file.is_file():
        return Config()
    data = tomllib.loads(file.read_text())
    known = {f.name for f in fields(Config)}
    stray = sorted(set(data) - known)
    if stray:
        raise SystemExit(f"briefpane: unknown keys in {file}: {', '.join(stray)}")
    cfg = Config(**{k: str(v) for k, v in data.items()})
    for key in ("agent_height", "pane_width"):
        value = getattr(cfg, key)
        if not (value.endswith("%") and value[:-1].isdigit() and 10 <= int(value[:-1]) <= 90):
            raise SystemExit(f"briefpane: {key} is a percentage from 10% to 90%, not {value!r}")
    if cfg.layout not in LAYOUTS:
        raise SystemExit(f"briefpane: layout must be one of {', '.join(LAYOUTS)}")
    return cfg
