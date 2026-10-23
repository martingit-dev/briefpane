import { MultiFileDiff } from "@pierre/diffs/react";
import DOMPurify from "dompurify";
import { marked } from "marked";
import { useEffect, useMemo, useState } from "react";
import { approvePlan, brief, type Card, changes, type FileChange, type Pane } from "./tmux";

const REFRESH_MS = 3000;
const STATE_LABEL = { "needs-you": "Needs you", working: "Working", done: "Ready" } as const;
const DIFF_OPTIONS = { theme: { dark: "min-dark", light: "min-light" }, themeType: "dark", diffStyle: "unified", overflow: "wrap" } as const;

/** The agent's last answer, rendered, and the repo's uncommitted changes. */
export function RightPanel({ pane, card, reloadCard }: { pane: Pane; card: Card | null; reloadCard: () => void }) {
  const [tab, setTab] = useState<"brief" | "card" | "changes">("brief");
  const [approveError, setApproveError] = useState<string | null>(null);
  const [answer, setAnswer] = useState<string | null>(null);
  const [files, setFiles] = useState<FileChange[]>([]);

  useEffect(() => {
    let live = true;
    const load = () => {
      if (pane.agent) brief(pane.pane_id).then((a) => live && setAnswer(a));
      changes(pane.path).then((f) => live && setFiles(f));
    };
    load();
    const t = setInterval(load, REFRESH_MS);
    return () => {
      live = false;
      clearInterval(t);
    };
  }, [pane.pane_id, pane.path, pane.agent]);

  const html = useMemo(() => (answer ? DOMPurify.sanitize(marked.parse(answer, { async: false })) : ""), [answer]);
  const state = pane.state?.state;

  return (
    <section className="right">
      <nav className="rtabs">
        <button type="button" className={tab === "brief" ? "on" : ""} onClick={() => setTab("brief")}>
          Brief
          {state === "needs-you" && <span className="n">1</span>}
        </button>
        <button type="button" className={tab === "card" ? "on" : ""} onClick={() => setTab("card")}>
          Card
          {card && !card.meta.plan_approved && card.plan && <span className="n">1</span>}
        </button>
        <button type="button" className={tab === "changes" ? "on" : ""} onClick={() => setTab("changes")}>
          Changes {files.length > 0 && <span className="faint">{files.length}</span>}
        </button>
      </nav>
      <div className="rbody">
        {tab === "brief" ? (
          <>
            {state && (
              <div className={`ask ${state}`}>
                <span className={`dot ${state}`} />
                <span>{pane.state?.message || STATE_LABEL[state]}</span>
              </div>
            )}
            {html ? (
              // Sanitised above: transcripts can quote fetched HTML.
              <article className="md" dangerouslySetInnerHTML={{ __html: html }} />
            ) : (
              <p className="faint">{pane.agent ? "No answer recorded yet." : "Not an agent pane."}</p>
            )}
          </>
        ) : tab === "card" ? (
          card ? (
            <div className="card">
              <section>
                <div className="cap">Define</div>
                <p>{card.define || <span className="faint">Not set.</span>}</p>
              </section>
              <section>
                <div className="cap">
                  Plan{" "}
                  {card.meta.plan_approved ? (
                    <span className="faint">· approved {card.meta.plan_approved.replace("T", " ")}</span>
                  ) : null}
                </div>
                {card.plan ? <pre className="plan">{card.plan}</pre> : <p className="faint">Not written.</p>}
                {card.plan && !card.meta.plan_approved && (
                  <div className="gatebar">
                    <span>File edits are blocked until you approve.</span>
                    <button
                      type="button"
                      className="btn pri"
                      onClick={() =>
                        approvePlan(pane.path)
                          .then(() => {
                            setApproveError(null);
                            reloadCard();
                          })
                          .catch((e) => setApproveError(String(e)))
                      }
                    >
                      Approve plan
                    </button>
                  </div>
                )}
                {approveError && <p className="err">{approveError}</p>}
              </section>
              <section>
                <div className="cap">
                  Proof <span className="faint">{card.proof.filter((p) => p.done).length} of {card.proof.length}</span>
                </div>
                {card.proof.length ? (
                  card.proof.map((p, i) => (
                    <div className="proof" key={`${i}-${p.claim}`}>
                      <span className={`dot ${p.done ? "done" : "idle"}`} />
                      <span>
                        {p.claim}
                        {p.evidence && <span className="mono faint"> {p.evidence}</span>}
                      </span>
                    </div>
                  ))
                ) : (
                  <p className="faint">None named.</p>
                )}
              </section>
              {card.next && (
                <section>
                  <div className="cap">Next</div>
                  <p>{card.next}</p>
                </section>
              )}
            </div>
          ) : (
            <p className="faint">
              No card for this branch. The agent starts one with <span className="mono">briefpane-card define</span>.
            </p>
          )
        ) : files.length ? (
          files.map((f) => (
            <div className="file" key={f.name} data-file={f.name}>
              {f.old === null && f.new === null ? (
                <p className="faint mono">
                  {f.status} {f.name} · too large to show
                </p>
              ) : (
                <MultiFileDiff
                  oldFile={{ name: f.name, contents: f.old ?? "" }}
                  newFile={{ name: f.name, contents: f.new ?? "" }}
                  options={DIFF_OPTIONS}
                />
              )}
            </div>
          ))
        ) : (
          <p className="faint">No uncommitted changes.</p>
        )}
      </div>
    </section>
  );
}
