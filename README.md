# briefpane

A side pane for coding agents. Your agent runs on the left as usual; briefpane
runs on the right and keeps the brief: the goal, the latest answer, what is
waiting on you, each turn folded to its answer, and the files the agent touched.

It only reads the transcript the agent already writes, so it makes no model
calls and costs nothing to run. Claude Code and Codex are supported.

## Install

```bash
uv tool install git+https://github.com/martingit-dev/briefpane
```

Needs Python 3.11+ and tmux.

## Use

```bash
briefpane claude            # start Claude Code with the pane beside it
briefpane codex             # same for Codex
briefpane claude --resume   # anything after the agent name goes to the agent
briefpane view claude       # just the pane, on the newest session in this directory
briefpane view codex --file ~/.codex/sessions/2026/10/05/rollout-....jsonl
```

Inside tmux the pane splits off the current window; outside it, briefpane
opens a new tmux session with both.

Keys: `t` theme, `f` files pane, `End` latest turn, `q` quit.

Themes: `matrix` (default), `dark`, `light`. Pick one with `--theme`, or cycle with `t`.

## Structured replies

briefpane shows any reply, leading with its first line. It reads best when the
agent answers in labelled lines, which you can ask for in your agent's
instructions:

```
answer: the outcome, or the decision you owe
why:    the mechanism in plain words
proof:  one pointer: file:line, session id, count
you:    what you do next, or the one question
more:   1 topic · 2 topic
```

The band at the top shows the session's first prompt as the goal, and the
latest `answer:` and `you:` lines.

## Adding an agent

An adapter tells briefpane how to start an agent, where it writes transcripts,
and what each record means. Subclass `Adapter` and register it:

```python
from briefpane.adapters.base import Adapter
from briefpane.model import AssistantText, ToolUse, UserText


class MyAgentAdapter(Adapter):
    name = "myagent"
    command = "myagent"

    def transcripts(self, cwd): ...  # paths of this agent's session files for cwd

    def parse(
        self, record
    ): ...  # [UserText(...), AssistantText(...), ToolUse(name, summary, files)]
```

Then add it to `ADAPTERS` in `src/briefpane/adapters/__init__.py`. See
`claude.py` and `codex.py` for real ones.

## Develop

```bash
uv sync
uv run pytest
uv run ruff check . && uv run ruff format --check .
```

## License

MIT
