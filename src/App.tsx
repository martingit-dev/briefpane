import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useMemo, useState } from "react";
import { PaneView } from "./PaneView";
import { RightPanel } from "./RightPanel";
import { listPanes, newTerminal, type Pane, PHASES, statusOf, taskOf } from "./tmux";
import { useCard } from "./useCard";

const repoOf = (p: Pane) => p.path.split("/").filter(Boolean).pop() ?? p.path;

// Sidebar order: what needs you first, shells last.
const GROUPS = [
  ["needs-you", "Needs you"],
  ["working", "Working"],
  ["done", "Ready"],
  ["idle", "Agents"],
  ["service", "Services"],
  ["terminal", "Terminals"],
] as const;

export default function App() {
  const [panes, setPanes] = useState<Pane[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [collapsed, setCollapsed] = useState(false);
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
  const [cardValue, reloadCard] = useCard(pane?.path);

  const openTerminal = () =>
    newTerminal(pane?.session ?? null, pane?.path ?? "~")
      .then((id) => {
        refresh();
        setSelected(id);
      })
      .catch((e) => setError(String(e)));

  const groups = useMemo(
    () =>
      GROUPS.map(([status, label]) => ({ status, label, items: panes.filter((p) => statusOf(p) === status) })).filter(
        (g) => g.items.length,
      ),
    [panes],
  );

  return (
    <div className={`app ${collapsed ? "collapsed" : ""}`}>
      <aside className="side">
        <div className="sidehead">
          {!collapsed && (
            <span className="brand">
              <svg aria-hidden="true" viewBox="0 0 64 64" width="16" height="16">
                <path
                  fill="currentColor"
                  fillRule="evenodd"
                  d="M11 16L21 6V24H43A10 10 0 0 1 53 34V48A10 10 0 0 1 43 58H11Z M21 34H43V48H21Z"
                />
              </svg>
              Briefpane
            </span>
          )}
          {!collapsed && (
            <button className="iconbtn newterm" type="button" title="New terminal" onClick={openTerminal}>
              <svg
                aria-hidden="true"
                viewBox="0 0 24 24"
                width="15"
                height="15"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.6"
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <path d="M5 12h14" />
                <path d="M12 5v14" />
              </svg>
            </button>
          )}
          <button
            className="iconbtn collapse"
            type="button"
            onClick={() => setCollapsed((c) => !c)}
            title="Toggle sidebar"
          >
            <svg
              aria-hidden="true"
              viewBox="0 0 16 16"
              width="14"
              height="14"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.3"
            >
              <rect x="1.5" y="2.5" width="13" height="11" rx="2" />
              <path d="M6 2.5v11" />
            </svg>
          </button>
        </div>
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
                  data-pane={p.pane_id}
                  className={`task ${p.pane_id === selected ? "on" : ""}`}
                  onClick={() => setSelected(p.pane_id)}
                >
                  <span className={`dot ${statusOf(p)}`} />
                  <span className="body">
                    <span className="t">{p.agent ? taskOf(p) : p.window_name}</span>
                    <span className="s">
                      {p.state?.state === "needs-you" && p.state.message
                        ? p.state.message
                        : `${p.agent ?? p.command} · ${p.agent ? repoOf(p) : p.session}`}
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
              <div className="title">
                <b>{pane.agent ? taskOf(pane) : pane.window_name}</b>
                <span className="mono faint">
                  {pane.session}:{pane.window_index} · {pane.path.replace(/^\/home\/[^/]+/, "~")}
                </span>
              </div>
              {cardValue && (
                <div className="phase">
                  {PHASES.map((ph) => {
                    const now = PHASES.indexOf(cardValue.meta.phase as (typeof PHASES)[number]);
                    const i = PHASES.indexOf(ph);
                    return (
                      <span key={ph} className={i === now ? "on" : i < now ? "done" : ""}>
                        {ph}
                      </span>
                    );
                  })}
                </div>
              )}
              <button className="iconbtn mic" type="button" title="Voice, transcribed by jane (not wired yet)" disabled>
                <svg
                  aria-hidden="true"
                  viewBox="0 0 24 24"
                  width="15"
                  height="15"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.6"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                >
                  <path d="M12 19v3" />
                  <path d="M19 10v2a7 7 0 0 1-14 0v-2" />
                  <rect x="9" y="2" width="6" height="13" rx="3" />
                </svg>
              </button>
            </header>
            <div className="termwrap">
              <PaneView key={pane.pane_id} pane={pane} />
            </div>
          </>
        ) : (
          <div className="empty">Pick a pane.</div>
        )}
      </main>
      {pane && <RightPanel pane={pane} card={cardValue} reloadCard={reloadCard} />}
    </div>
  );
}
