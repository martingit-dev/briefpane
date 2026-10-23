import { listen } from "@tauri-apps/api/event";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import { useEffect, useRef } from "react";
import { attach, capture, type Output, type Pane, resize, sendKeys } from "./tmux";

// tmux needs a moment to resize the window and the program to redraw
// before the snapshot is worth taking.
const SETTLE_MS = 120;

/** A live view of one tmux pane that fills its box, like a terminal. */
export function PaneView({ pane }: { pane: Pane }) {
  const host = useRef<HTMLDivElement>(null);
  const { session, pane_id } = pane;

  useEffect(() => {
    const el = host.current;
    if (!el) return;
    const term = new Terminal({
      fontSize: 13,
      lineHeight: 1.15,
      fontFamily: "'JetBrains Mono', 'Geist Mono', ui-monospace, monospace",
      theme: { background: "#0f0e0c", foreground: "#d8d4cd", cursor: "#d8d4cd" },
      scrollback: 5000,
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(el);
    fit.fit();

    let live = true;
    let unlisten: (() => void) | undefined;
    const buffered: Uint8Array[] = [];
    let primed = false;

    // Subscribe before the snapshot so no output falls between the two.
    listen<Output>("pane-output", (e) => {
      if (e.payload.pane !== pane_id) return;
      const bytes = new Uint8Array(e.payload.data);
      if (primed) term.write(bytes);
      else buffered.push(bytes);
    }).then(async (u) => {
      if (!live) return u();
      unlisten = u;
      await attach(session);
      await resize(session, term.cols, term.rows);
      await new Promise((r) => setTimeout(r, SETTLE_MS));
      const s = await capture(pane_id);
      if (!live) return;
      const crlf = (t: string) => t.replace(/\n$/, "").replaceAll("\n", "\r\n");
      if (s.history.trim()) term.write(`${crlf(s.history)}\r\n`);
      term.write(crlf(s.text));
      term.write(`\x1b[${s.cursor_y + 1};${s.cursor_x + 1}H`);
      for (const b of buffered) term.write(b);
      primed = true;
    });

    let pending: ReturnType<typeof setTimeout> | undefined;
    const observer = new ResizeObserver(() => {
      clearTimeout(pending);
      pending = setTimeout(() => {
        fit.fit();
        void resize(session, term.cols, term.rows).catch(() => {});
      }, 80);
    });
    observer.observe(el);

    const input = term.onData((d) => void sendKeys(session, pane_id, d));
    term.focus();

    return () => {
      live = false;
      clearTimeout(pending);
      unlisten?.();
      observer.disconnect();
      input.dispose();
      term.dispose();
    };
  }, [session, pane_id]);

  return <div className="paneview" ref={host} />;
}
