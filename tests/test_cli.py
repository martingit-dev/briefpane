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


def test_claude_runs_under_a_session_id_so_the_pane_follows_only_that_session():
    from briefpane.adapters.claude import ClaudeAdapter

    a = ClaudeAdapter(root=Path("/p"))
    args, transcript = a.prepare(["--model", "opus"], Path("/r"))
    assert args[0] == "--session-id" and args[2:] == ["--model", "opus"]
    assert transcript == Path(f"/p/-r/{args[1]}.jsonl")
    assert a.prepare(["--resume"], Path("/r")) == (["--resume"], None)


def test_find_skips_sessions_that_existed_before_the_launch(tmp_path):
    from briefpane.adapters.claude import ClaudeAdapter, project_dir

    folder = project_dir(Path("/r"), tmp_path)
    folder.mkdir(parents=True)
    other = folder / "other.jsonl"
    other.write_text("{}\n")
    a = ClaudeAdapter(root=tmp_path)
    existing = frozenset(a.transcripts(Path("/r")))
    assert a.find(Path("/r"), exclude=existing) is None
    mine = folder / "mine.jsonl"
    mine.write_text("{}\n")
    other.write_text("{}\n{}\n")  # the busy session keeps writing
    assert a.find(Path("/r"), exclude=existing) == mine
