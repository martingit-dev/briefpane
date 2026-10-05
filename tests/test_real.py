"""Read whatever real transcripts this machine has; skipped where there are none."""

from pathlib import Path

import pytest

from briefpane.adapters import get
from briefpane.model import Session


def _newest(pattern: str) -> Path | None:
    files = sorted(Path.home().glob(pattern), key=lambda p: p.stat().st_mtime)
    return files[-1] if files else None


@pytest.mark.parametrize(
    "agent,pattern",
    [("claude", ".claude/projects/*/*.jsonl"), ("codex", ".codex/sessions/*/*/*/rollout-*.jsonl")],
)
def test_a_real_transcript_folds_into_turns(agent, pattern):
    path = _newest(pattern)
    if path is None:
        pytest.skip(f"no {agent} transcript here")
    adapter, session = get(agent), Session()
    for line in path.read_text(errors="replace").splitlines():
        for event in adapter.parse_line(line):
            session.add(event)
    assert session.turns and session.goal
