import { listen } from "@tauri-apps/api/event";
import { Terminal } from "@xterm/xterm";
import { useEffect, useRef } from "react";
import { attach, capture, type Output, type Pane, sendKeys } from "./tmux";

// Monospace cell size per px of font size, used to fit the pane to the view.
const CELL_W = 0.6;
const CELL_H = 1.2;

const fit = (el: HTMLElement, cols: number, rows: number) =>
  Math.max(9, Math.min(15, Math.floor(Math.min(el.clientWidth / (cols * CELL_W), el.clientHeight / (rows * CELL_H)))));

/** A live view of one tmux pane. Sized to the pane, never the reverse. */
export function PaneView({ pane }: { pane: Pane }) {
  const host = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const el = host.current;
    if (!el) return;
    const term = new Terminal({
      cols: pane.width,
      rows: pane.height,
      fontSize: fit(el, pane.width, pane.height),
      fontFamily: "'JetBrains Mono', 'Geist Mono', ui-monospace, monospace",
      theme: { background: "#0f0e0c", foreground: "#d8d4cd", cursor: "#d8d4cd" },
      scrollback: 0,
    });
    term.open(el);

    let live = true;
    let unlisten: (() => void) | undefined;
    const buffered: Uint8Array[] = [];
    let primed = false;

    // Subscribe before the snapshot so no output falls between the two.
    listen<Output>("pane-output", (e) => {
      if (e.payload.pane !== pane.pane_id) return;
      const bytes = new Uint8Array(e.payload.data);
      if (primed) term.write(bytes);
      else buffered.push(bytes);
    }).then((u) => {
      if (!live) return u();
      unlisten = u;
      return attach(pane.session)
        .then(() => capture(pane.pane_id))
        .then((s) => {
          if (!live) return;
          term.write(s.text.replace(/\n$/, "").replaceAll("\n", "\r\n"));
          term.write(`\x1b[${s.cursor_y + 1};${s.cursor_x + 1}H`);
          for (const b of buffered) term.write(b);
          primed = true;
        });
    });

    const resize = new ResizeObserver(() => {
      term.options.fontSize = fit(el, pane.width, pane.height);
    });
    resize.observe(el);

    const input = term.onData((d) => void sendKeys(pane.session, pane.pane_id, d));
    term.focus();

    return () => {
      live = false;
      unlisten?.();
      resize.disconnect();
      input.dispose();
      term.dispose();
    };
  }, [pane.session, pane.pane_id, pane.width, pane.height]);

  return <div className="paneview" ref={host} />;
}
