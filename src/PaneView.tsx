import { listen } from "@tauri-apps/api/event";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import { useEffect, useRef } from "react";
import { attach, capture, type Output, type Pane, resize, sendKeys } from "./tmux";

const BASE_FONT = 13;
const MIN_FONT = 9;
// How long to wait for tmux to settle a resize before taking the snapshot.
const SETTLE_MS = 50;
const SETTLE_TRIES = 20;

/**
 * A live view of one tmux pane. The pane's own size is the truth: the view
 * draws exactly that many columns and rows and scales the font to fit, so
 * output written for the pane lands on the same cells here. A pane in a
 * split, or one sized by another client, would otherwise wrap differently
 * and draw over the wrong rows.
 */
export function PaneView({ pane }: { pane: Pane }) {
  const host = useRef<HTMLDivElement>(null);
  const termRef = useRef<{ term: Terminal; fitFont: () => void } | null>(null);
  const { session, pane_id } = pane;

  // The terminal is built once per pane; the size effect below follows resizes.
  useEffect(() => {
    const el = host.current;
    if (!el) return;
    const term = new Terminal({
      cols: pane.width,
      rows: pane.height,
      fontSize: BASE_FONT,
      lineHeight: 1.15,
      fontFamily: "'JetBrains Mono', 'Geist Mono', ui-monospace, monospace",
      theme: { background: "#0f0e0c", foreground: "#d8d4cd", cursor: "#d8d4cd" },
      scrollback: 5000,
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(el);
    // The e2e test reads the buffer: WebKit stops painting a window that is off screen.
    (window as unknown as { __bpTerm: Terminal }).__bpTerm = term;

    // What the view holds at the base font: the window size we ask tmux for.
    const wanted = () => {
      const k = (term.options.fontSize ?? BASE_FONT) / BASE_FONT;
      const d = fit.proposeDimensions();
      if (!d) return null;
      return { cols: Math.max(20, Math.floor(d.cols * k)), rows: Math.max(5, Math.floor(d.rows * k)) };
    };
    // Shrink the font until the pane's columns and rows fit the view.
    const fitFont = () => {
      let size = BASE_FONT;
      term.options.fontSize = size;
      let d = fit.proposeDimensions();
      while (d && size > MIN_FONT && (d.cols < term.cols || d.rows < term.rows)) {
        size -= 0.5;
        term.options.fontSize = size;
        d = fit.proposeDimensions();
      }
    };
    termRef.current = { term, fitFont };
    fitFont();

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
    })
      .then(async (u) => {
        if (!live) return u();
        unlisten = u;
        await attach(session);
        const w = wanted();
        if (w) await resize(session, w.cols, w.rows);
        // Wait for the pane's size to stop changing. In a split it never
        // equals what we asked for, so "stable" is the signal, not "equal".
        let s = await capture(pane_id);
        for (let i = 0; i < SETTLE_TRIES; i++) {
          await new Promise((r) => setTimeout(r, SETTLE_MS));
          buffered.length = 0; // the next snapshot already contains it
          const next = await capture(pane_id);
          const stable = next.width === s.width && next.height === s.height;
          s = next;
          if (stable) break;
        }
        if (!live) return;
        term.resize(s.width, s.height);
        fitFont();
        const lines = (t: string) => t.replace(/\n$/, "").split("\n");
        if (s.history.trim()) term.write(`${lines(s.history).join("\r\n")}\r\n`);
        // Scroll history out of view, then paint each screen row in place.
        term.write("\r\n".repeat(term.rows));
        for (const [row, line] of lines(s.text).entries()) term.write(`\x1b[${row + 1};1H${line}\x1b[0m`);
        term.write(`\x1b[${s.cursor_y + 1};${s.cursor_x + 1}H`);
        for (const b of buffered) term.write(b);
        primed = true;
      })
      .catch((err) => {
        // Surfaced on the element so a failed attach is visible, not a blank pane.
        el.dataset.error = String(err);
      });

    // When the view changes size, ask tmux for a matching window; the pane's
    // new size comes back through the pane list and the effect below.
    let pending: ReturnType<typeof setTimeout> | undefined;
    const observer = new ResizeObserver(() => {
      clearTimeout(pending);
      pending = setTimeout(() => {
        fitFont();
        const w = wanted();
        if (w) void resize(session, w.cols, w.rows).catch(() => {});
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
      termRef.current = null;
      term.dispose();
    };
  }, [session, pane_id]);

  // Follow the pane when tmux resizes it, from us or from another client.
  useEffect(() => {
    const t = termRef.current;
    if (!t || (t.term.cols === pane.width && t.term.rows === pane.height)) return;
    t.term.resize(pane.width, pane.height);
    t.fitFont();
  }, [pane.width, pane.height]);

  return <div className="paneview" ref={host} />;
}
