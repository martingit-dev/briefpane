import { listen } from "@tauri-apps/api/event";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import { useEffect, useRef, useState } from "react";
import { attach, capture, log, type Pane, paneSize, resize, type Snapshot, sendKeys, unwatch, watch } from "./tmux";

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
  // Bumped when the session's control client ends, which reopens the view.
  const [epoch, setEpoch] = useState(0);
  useEffect(() => {
    const un = listen<string>("tmux-detached", (e) => {
      if (e.payload === session) setEpoch((n) => n + 1);
    });
    return () => void un.then((u) => u());
  }, [session]);

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
    if (__BP_TEST_HOOKS__) (window as unknown as { __bpTerm: Terminal }).__bpTerm = term;

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
    let watchId: number | undefined;
    const buffered: Uint8Array[] = [];
    let primed = false;
    // Resizes wait for the control client; before it exists tmux has no client to size.
    let attached = false;
    const opened = performance.now();

    const open = async () => {
      try {
        await attach(session);
        attached = true;
        // Watch before the snapshot so no output falls between the two.
        const id = await watch(session, pane_id, (bytes) => {
          if (!live) return;
          if (primed) term.write(bytes);
          else buffered.push(bytes);
        });
        if (!live) return void unwatch(id).catch(() => {});
        watchId = id;
        const w = wanted();
        if (w) await resize(session, w.cols, w.rows);
        // Wait for the pane's size to stop changing. In a split it never
        // equals what we asked for, so "stable" is the signal, not "equal".
        let size = await paneSize(pane_id);
        for (let i = 0; i < SETTLE_TRIES && live; i++) {
          await new Promise((r) => setTimeout(r, SETTLE_MS));
          const next = await paneSize(pane_id);
          const stable = next.width === size.width && next.height === size.height;
          size = next;
          if (stable) break;
        }
      } catch (err) {
        fail("attach", err);
        return;
      }
      if (!live) return;
      let s: Snapshot;
      try {
        buffered.length = 0; // the snapshot already contains it
        s = await capture(pane_id);
      } catch (err) {
        fail("capture", err);
        return;
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
      buffered.length = 0;
      primed = true;
      el.dataset.ready = pane_id;
      log("perf", "pane-open", { ms: Math.round(performance.now() - opened), pane: pane_id });
    };
    const fail = (stage: string, err: unknown) => {
      if (!live) return;
      // Surfaced on the element so a failed attach is visible, not a blank pane.
      el.dataset.error = String(err);
      // A view left on a session that ended has nothing to reopen.
      if (!String(err).startsWith("no session")) {
        log("error", `pane ${stage} failed`, { session, pane: pane_id, error: String(err) });
      }
    };
    void open();

    // When the view changes size, ask tmux for a matching window; the pane's
    // new size comes back through the pane list and the effect below.
    let pending: ReturnType<typeof setTimeout> | undefined;
    const observer = new ResizeObserver(() => {
      clearTimeout(pending);
      pending = setTimeout(() => {
        fitFont();
        const w = wanted();
        if (w && attached) void resize(session, w.cols, w.rows).catch(() => {});
      }, 80);
    });
    observer.observe(el);

    // With `window-size latest`, typing in another terminal on this session
    // takes the size back. Using the view here claims it again.
    // Asked at most once a second; tmux ignores a size it already has.
    let claimed = 0;
    const claim = () => {
      if (!attached || Date.now() - claimed < 1000) return;
      claimed = Date.now();
      const w = wanted();
      if (w) void resize(session, w.cols, w.rows).catch(() => {});
    };
    const input = term.onData((d) => {
      claim();
      sendKeys(session, pane_id, d).catch(() => {});
    });
    term.textarea?.addEventListener("focus", claim);
    term.focus();

    return () => {
      live = false;
      clearTimeout(pending);
      if (watchId !== undefined) void unwatch(watchId).catch(() => {});
      observer.disconnect();
      input.dispose();
      term.textarea?.removeEventListener("focus", claim);
      termRef.current = null;
      // The host outlives a reopen; its markers describe the view being torn down.
      delete el.dataset.ready;
      delete el.dataset.error;
      term.dispose();
    };
  }, [session, pane_id, epoch]);

  // Follow the pane when tmux resizes it, from us or from another client.
  useEffect(() => {
    const t = termRef.current;
    if (!t || (t.term.cols === pane.width && t.term.rows === pane.height)) return;
    t.term.resize(pane.width, pane.height);
    t.fitFont();
  }, [pane.width, pane.height]);

  return <div className="paneview" ref={host} />;
}
