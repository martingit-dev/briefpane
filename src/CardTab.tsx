import { useState } from "react";
import { approvePlan, type Card, dispose, type Pane, sendKeys, startReview } from "./tmux";

/** What the building agent is told: the findings the human marked fix. Control
 * characters are dropped, since this is typed into the agent as keystrokes. */
const fixMessage = (fixes: Card["review"]) =>
  `Fix these review findings, then re-run the proof and update the card: ${fixes
    .map((r) => `${r.id} (${r.severity}) ${r.summary}`)
    .join("; ")}`.replace(/\p{Cc}+/gu, " ");

/** The task card: definition, plan gate, proof and review verdicts. */
export function CardTab({ pane, card, reloadCard }: { pane: Pane; card: Card | null; reloadCard: () => void }) {
  const [actionError, setActionError] = useState<string | null>(null);
  const fixes = card?.review.filter((r) => r.state === "fix") ?? [];

  if (!card)
    return (
      <p className="faint">
        No card for this branch. The agent starts one with <span className="mono">briefpane-card define</span>.
      </p>
    );

  return (
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
                    setActionError(null);
                    reloadCard();
                  })
                  .catch((e) => setActionError(String(e)))
              }
            >
              Approve plan
            </button>
          </div>
        )}
        {actionError && <p className="err">{actionError}</p>}
      </section>
      <section>
        <div className="cap">
          Proof{" "}
          <span className="faint">
            {card.proof.filter((p) => p.done).length} of {card.proof.length}
          </span>
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
      <section>
        <div className="cap">
          Review{" "}
          {card.review.length > 0 && (
            <span className="faint">{card.review.filter((r) => r.state === "open").length} open</span>
          )}
        </div>
        {card.review.map((r) => (
          <div className={`finding ${r.state}`} key={r.id}>
            <span className="mono faint">
              {r.id} {r.severity}
            </span>
            <span className="sum">{r.summary}</span>
            <span className="verdicts">
              {(["fix", "defer", "reject"] as const).map((v) => (
                <button
                  type="button"
                  key={v}
                  className={`btn ${r.state === v ? "pri" : ""}`}
                  onClick={() =>
                    dispose(pane.path, r.id, v)
                      .then(reloadCard)
                      .catch((e) => setActionError(String(e)))
                  }
                >
                  {v}
                </button>
              ))}
            </span>
          </div>
        ))}
        <div className="reviewbar">
          {card.meta.plan_approved && pane.agent && (
            <button
              type="button"
              className="btn"
              onClick={() =>
                startReview(pane.session, pane.path, pane.agent ?? "", pane.pane_id)
                  .then(reloadCard)
                  .catch((e) => setActionError(String(e)))
              }
            >
              Start review
            </button>
          )}
          {fixes.length > 0 && card.meta.builder && (
            <button
              type="button"
              className="btn pri"
              onClick={() =>
                sendKeys(pane.session, card.meta.builder ?? "", `${fixMessage(fixes)}\r`).catch((e) =>
                  setActionError(String(e)),
                )
              }
            >
              Send {fixes.length} {fixes.length === 1 ? "fix" : "fixes"} to the agent
            </button>
          )}
        </div>
      </section>
      {card.next && (
        <section>
          <div className="cap">Next</div>
          <p>{card.next}</p>
        </section>
      )}
    </div>
  );
}
