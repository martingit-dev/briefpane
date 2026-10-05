import json
import os
from pathlib import Path

from briefpane.adapters.claude import ClaudeAdapter, project_dir
from briefpane.model import AssistantText, ToolUse, UserText

A = ClaudeAdapter()


def test_the_project_folder_replaces_every_non_alphanumeric():
    got = project_dir(Path("/home/mg/heya/programming/agent-daemon.wt_x"), Path("/p"))
    assert got == Path("/p/-home-mg-heya-programming-agent-daemon-wt-x")


def test_user_words_count_and_system_text_does_not():
    assert A.parse({"type": "user", "message": {"content": "review this"}}) == [
        UserText("review this")
    ]
    assert A.parse({"type": "user", "message": {"content": "<command-name>/clear"}}) == []
    interrupted = {"type": "user", "message": {"content": "[Request interrupted by user]"}}
    assert A.parse(interrupted) == []
    blocks = [
        {"type": "tool_result", "content": "x"},
        {"type": "text", "text": "<system-reminder>"},
    ]
    assert A.parse({"type": "user", "message": {"content": blocks}}) == []
    assert A.parse({"type": "user", "isMeta": True, "message": {"content": "hi"}}) == []


def test_assistant_text_and_tool_calls_with_their_files():
    record = {
        "type": "assistant",
        "message": {
            "content": [
                {"type": "text", "text": "Looking."},
                {"type": "tool_use", "name": "Read", "input": {"file_path": "/r/a.py"}},
                {"type": "tool_use", "name": "Write", "input": {"file_path": "/r/b.py"}},
                {
                    "type": "tool_use",
                    "name": "Bash",
                    "input": {"command": "ls", "description": "List"},
                },
            ]
        },
    }
    assert A.parse(record) == [
        AssistantText("Looking."),
        ToolUse("Read", "/r/a.py", (("/r/a.py", "read"),)),
        ToolUse("Write", "/r/b.py", (("/r/b.py", "edited"),)),
        ToolUse("Bash", "List"),
    ]


def test_a_subagent_record_is_left_out():
    assert A.parse({"type": "assistant", "isSidechain": True, "message": {"content": []}}) == []


def test_find_picks_the_newest_session_written_since(tmp_path):
    cwd = Path("/work/repo")
    folder = project_dir(cwd, tmp_path)
    folder.mkdir(parents=True)
    old, new = folder / "old.jsonl", folder / "new.jsonl"
    for i, p in enumerate((old, new)):
        p.write_text(json.dumps({"type": "user"}) + "\n")
        os.utime(p, (1000 + i, 1000 + i))
    adapter = ClaudeAdapter(root=tmp_path)
    assert adapter.find(cwd) == new
    assert adapter.find(cwd, since=5000) is None
