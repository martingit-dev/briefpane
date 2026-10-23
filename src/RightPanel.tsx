import { lazy, Suspense, useState } from "react";
import { BriefTab } from "./BriefTab";
import { CardTab } from "./CardTab";
import type { Card, Pane } from "./tmux";

// Lazy so the diff renderer and its highlighter stay out of the main chunk.
const ChangesTab = lazy(() => import("./ChangesTab"));

/** The agent's last answer, its task card, and the repo's uncommitted changes. */
export function RightPanel({ pane, card, reloadCard }: { pane: Pane; card: Card | null; reloadCard: () => void }) {
  const [tab, setTab] = useState<"brief" | "card" | "changes">("brief");
  const [changed, setChanged] = useState<number | null>(null);
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
          Changes {changed ? <span className="faint">{changed}</span> : null}
        </button>
      </nav>
      <div className="rbody">
        {tab === "brief" ? (
          <BriefTab pane={pane} />
        ) : tab === "card" ? (
          <CardTab pane={pane} card={card} reloadCard={reloadCard} />
        ) : (
          <Suspense fallback={<p className="faint">Loading…</p>}>
            <ChangesTab path={pane.path} onCount={setChanged} />
          </Suspense>
        )}
      </div>
    </section>
  );
}
