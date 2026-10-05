import json
from pathlib import Path

from briefpane.adapters.codex import CodexAdapter
from briefpane.model import AssistantText, ToolUse, UserText

A = CodexAdapter(root=Path("/nonexistent"))


def item(payload: dict) -> dict:
    return {"type": "response_item", "payload": payload}


def test_user_words_count_and_injected_context_does_not():
    msg = {
        "type": "message",
        "role": "user",
        "content": [
            {"type": "input_text", "text": "# AGENTS.md instructions for /home/mg"},
            {"type": "input_text", "text": "<environment_context>"},
            {"type": "input_text", "text": "review the campaign ui"},
        ],
    }
    assert A.parse(item(msg)) == [UserText("review the campaign ui")]


def test_assistant_messages_and_developer_messages():
    out = {
        "type": "message",
        "role": "assistant",
        "content": [{"type": "output_text", "text": "Done."}],
    }
    assert A.parse(item(out)) == [AssistantText("Done.")]
    dev = {"type": "message", "role": "developer", "content": [{"type": "input_text", "text": "x"}]}
    assert A.parse(item(dev)) == []


def test_a_patch_names_the_files_it_edits_adds_and_deletes():
    patch = (
        'const patch = "*** Begin Patch\\n*** Update File: /r/app.tsx\\n@@\\n'
        '*** Add File: /r/new.ts\\n+x\\n*** Delete File: /r/old.ts\\n*** End Patch";'
    )
    call = {"type": "custom_tool_call", "name": "exec", "input": patch}
    [use] = A.parse(item(call))
    assert use.files == (("/r/app.tsx", "edited"), ("/r/new.ts", "new"), ("/r/old.ts", "deleted"))
    assert use.summary == "patch app.tsx, new.ts, old.ts"


def test_a_shell_read_names_the_file_it_reads():
    raw = "const r = await tools.exec_command({cmd:\"cat /r/SKILL.md && sed -n '1,40p' /r/x.py\"});"
    [use] = A.parse(item({"type": "custom_tool_call", "name": "exec", "input": raw}))
    assert use == ToolUse(
        "exec",
        "cat /r/SKILL.md && sed -n '1,40p' /r/x.py",
        (("/r/SKILL.md", "read"), ("/r/x.py", "read")),
    )


def test_transcripts_are_matched_by_the_session_cwd(tmp_path):
    day = tmp_path / "2026" / "10" / "05"
    day.mkdir(parents=True)
    for name, cwd in (("rollout-a.jsonl", "/work/repo"), ("rollout-b.jsonl", "/elsewhere")):
        meta = {"type": "session_meta", "payload": {"cwd": cwd}}
        (day / name).write_text(json.dumps(meta) + "\n")
    found = CodexAdapter(root=tmp_path).transcripts(Path("/work/repo"))
    assert [p.name for p in found] == ["rollout-a.jsonl"]
