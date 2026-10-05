from briefpane.model import AssistantText, Session, ToolUse, UserText, parse_reply


def test_a_labelled_reply_reads_as_its_labels():
    r = parse_reply(
        "answer: Yes, but your chat never reached it.\n"
        "why:    Studio mocks every tool,\n"
        "        so Hayley got no data.\n"
        "you:    Set tools to real."
    )
    assert r.answer == "Yes, but your chat never reached it."
    assert r.labels["why"] == "Studio mocks every tool, so Hayley got no data."
    assert r.you == "Set tools to real."
    assert r.is_structured


def test_bold_labels_count_as_labels():
    assert parse_reply("**answer:** done\n**you:** go").you == "go"


def test_an_unlabelled_reply_leads_with_its_first_line():
    r = parse_reply("It works now.\n\nThe cause was a stale port.")
    assert r.answer == "It works now." and r.body == "The cause was a stale port."


def test_text_before_a_tool_call_is_narration_not_the_reply():
    s = Session()
    for e in (
        UserText("fix it"),
        AssistantText("Checking the log."),
        ToolUse("Bash", "grep error"),
        AssistantText("answer: fixed"),
    ):
        s.add(e)
    t = s.turns[-1]
    assert t.notes == ["Checking the log."] and t.reply.answer == "fixed"
    assert t.tool_summary() == "ran 1 tool: Bash"


def test_an_edit_outranks_a_later_read_and_files_follow_turns():
    s = Session()
    s.add(UserText("one"))
    s.add(ToolUse("Edit", "a", (("/r/a.py", "edited"),)))
    s.add(UserText("two"))
    s.add(ToolUse("Read", "a", (("/r/a.py", "read"),)))
    s.add(ToolUse("Read", "b", (("/r/b.py", "read"),)))
    assert s.files["/r/a.py"].verb == "edited"
    assert [f.path for f in s.files_this_turn()] == ["/r/b.py", "/r/a.py"]
    assert [f.path for f in s.files_changed()] == ["/r/a.py"]


def test_the_goal_is_the_first_prompt_and_now_the_latest_answer():
    s = Session()
    s.add(UserText("book a demo"))
    s.add(AssistantText("answer: offered two times\nyou: pick one"))
    assert s.goal == "book a demo"
    assert s.latest_reply.answer == "offered two times" and s.latest_reply.you == "pick one"
