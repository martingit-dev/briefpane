import DOMPurify from "dompurify";
import { marked } from "marked";
import { useMemo } from "react";
import { brief, type Pane } from "./tmux";
import { usePoll } from "./usePoll";

const REFRESH_MS = 3000;
const STATE_LABEL = { "needs-you": "Needs you", working: "Working", done: "Ready" } as const;

/** The agent's state and its last answer, rendered. */
export function BriefTab({ pane }: { pane: Pane }) {
  const { pane_id, agent } = pane;
  const { value: answer } = usePoll("brief", () => (agent ? brief(pane_id) : Promise.resolve(null)), REFRESH_MS, [
    pane_id,
    agent,
  ]);
  const html = useMemo(() => (answer ? DOMPurify.sanitize(marked.parse(answer, { async: false })) : ""), [answer]);
  const state = pane.state?.state;

  return (
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
        <p className="faint">{agent ? "No answer recorded yet." : "Not an agent pane."}</p>
      )}
    </>
  );
}
