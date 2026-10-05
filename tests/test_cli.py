from pathlib import Path

import pytest

from briefpane.cli import tmux_plan
from briefpane.config import Config, load

AGENT, VIEWER, CWD = ["claude"], ["python", "-m", "briefpane", "view", "claude"], Path("/r")


def test_bottom_puts_briefpane_above_and_the_agent_keeps_your_pane():
    before, final = tmux_plan("bottom", True, AGENT, VIEWER, CWD, Config(agent_height="30%"))
    assert before == [
        [
            "tmux",
            "split-window",
            "-v",
            "-b",
            "-d",
            "-l",
            "70%",
            "-c",
            "/r",
            "python -m briefpane view claude",
        ]
    ]
    assert final == ["claude"]


def test_side_splits_right_and_outside_tmux_starts_a_session():
    before, final = tmux_plan("side", False, AGENT, VIEWER, CWD, Config(pane_width="35%"))
    assert before == []
    assert final[:5] == ["tmux", "new-session", "-c", "/r", "claude"]
    assert final[5:9] == [";", "split-window", "-h", "-d"] and "35%" in final


def test_the_config_file_sets_defaults_and_rejects_typos(tmp_path):
    f = tmp_path / "config.toml"
    f.write_text('theme = "dark"\nlayout = "side"\n')
    assert load(f) == Config(theme="dark", layout="side")
    f.write_text('layuot = "side"\n')
    with pytest.raises(SystemExit, match="unknown keys"):
        load(f)
    f.write_text('agent_height = "12"\n')
    with pytest.raises(SystemExit, match="percentage"):
        load(f)
    assert load(tmp_path / "missing.toml") == Config()
