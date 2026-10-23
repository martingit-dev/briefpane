import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useMemo, useState } from "react";
import { PaneView } from "./PaneView";
import { listPanes, type Pane, sendKeys, taskOf } from "./tmux";

const repoOf = (p: Pane) => p.path.split("/").filter(Boolean).pop() ?? p.path;

export default function App() {
  const [panes, setPanes] = useState<Pane[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [collapsed, setCollapsed] = useState(false);
  const [draft, setDraft] = useState("");
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    listPanes()
      .then((p) => {
        setPanes(p);
        setError(null);
      })
      .catch((e) => setError(String(e)));
  }, []);

  useEffect(() => {
    refresh();
    const t = setInterval(refresh, 3000);
    const un = listen("panes-changed", refresh);
    return () => {
      clearInterval(t);
      un.then((u) => u());
    };
  }, [refresh]);

  const pane = panes.find((p) => p.pane_id === selected) ?? null;

  // Agents first, then everything else, each grouped by session.
  const groups = useMemo(() => {
    const agents = panes.filter((p) => p.agent);
    const rest = panes.filter((p) => !p.agent);
    return [
      { label: "Agents", items: agents },
      { label: "Other panes", items: rest },
    ].filter((g) => g.items.length);
  }, [panes]);

  function submit(e: React.FormEvent) {
    e.preventDefault();
    if (!pane || !draft.trim()) return;
    void sendKeys(pane.session, pane.pane_id, `${draft}\r`);
    setDraft("");
  }

  return (
    <div className={`app ${collapsed ? "collapsed" : ""}`}>
      <aside className="side">
        <button className="iconbtn collapse" type="button" onClick={() => setCollapsed((c) => !c)} title="Toggle sidebar">
          <svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" strokeWidth="1.3">
            <rect x="1.5" y="2.5" width="13" height="11" rx="2" />
            <path d="M6 2.5v11" />
          </svg>
        </button>
        {!collapsed &&
          groups.map((g) => (
            <section key={g.label}>
              <div className="cap">
                {g.label} {g.items.length}
              </div>
              {g.items.map((p) => (
                <button
                  type="button"
                  key={p.pane_id}
                  className={`task ${p.pane_id === selected ? "on" : ""}`}
                  onClick={() => setSelected(p.pane_id)}
                >
                  <span className={`dot ${p.agent ? "agent" : ""}`} />
                  <span className="body">
                    <span className="t">{p.agent ? taskOf(p) : p.window_name}</span>
                    <span className="s">
                      {p.agent ?? p.command} · {p.agent ? repoOf(p) : p.session}
                    </span>
                  </span>
                </button>
              ))}
            </section>
          ))}
        {!collapsed && error && <p className="err">{error}</p>}
      </aside>

      <main className="mid">
        {pane ? (
          <>
            <header className="head">
              <b>{pane.agent ? taskOf(pane) : pane.window_name}</b>
              <span className="mono faint">
                {pane.session}:{pane.window_index} · {pane.path.replace(/^\/home\/[^/]+/, "~")}
              </span>
            </header>
            <div className="termwrap">
              <PaneView key={pane.pane_id} pane={pane} />
            </div>
            <form className="composer" onSubmit={submit}>
              <input
                value={draft}
                onChange={(e) => setDraft(e.target.value)}
                placeholder={`Message ${pane.agent ?? pane.command}…`}
              />
              <button className="iconbtn" type="button" title="Voice, transcribed by jane (not wired yet)" disabled>
                <svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round">
                  <rect x="5.5" y="1.5" width="5" height="8.5" rx="2.5" />
                  <path d="M3 7.5a5 5 0 0 0 10 0M8 12.5v2" />
                </svg>
              </button>
            </form>
          </>
        ) : (
          <div className="empty">Pick a pane.</div>
        )}
      </main>
    </div>
  );
}
