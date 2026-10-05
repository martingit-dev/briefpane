import json

from rich.console import Console

from briefpane.adapters.claude import ClaudeAdapter
from briefpane.app import BriefPane


def lines(*records: dict) -> str:
    return "".join(json.dumps(r) + "\n" for r in records)


async def test_the_pane_follows_a_transcript_as_it_grows(tmp_path):
    t = tmp_path / "s.jsonl"
    t.write_text(lines({"type": "user", "message": {"content": "book a demo"}}))
    app = BriefPane(ClaudeAdapter(), tmp_path, path=t)
    async with app.run_test(size=(140, 40)) as pilot:
        await pilot.pause(0.6)
        with t.open("a") as fh:
            fh.write(
                lines(
                    {
                        "type": "assistant",
                        "message": {
                            "content": [
                                {
                                    "type": "tool_use",
                                    "name": "Edit",
                                    "input": {"file_path": str(tmp_path / "a.py")},
                                },
                                {"type": "text", "text": "answer: booked\nyou: check your email"},
                            ]
                        },
                    }
                )
            )
        await pilot.pause(0.8)
        band = str(app.query_one("#band").render())
        console = Console(width=60, record=True)
        console.print(app.files_view)
        files = console.export_text()
        assert "book a demo" in band and "booked" in band and "check your email" in band
        assert "a.py" in files and "edited" in files


async def test_t_cycles_matrix_dark_light(tmp_path):
    app = BriefPane(ClaudeAdapter(), tmp_path, path=tmp_path / "none.jsonl")
    async with app.run_test() as pilot:
        assert app.theme == "matrix"
        await pilot.press("t")
        assert app.theme == "briefpane-dark"
        await pilot.press("t")
        assert app.theme == "briefpane-light"
