import { invoke } from "@tauri-apps/api/core";

export type Pane = {
  session: string;
  window_id: string;
  window_index: number;
  window_name: string;
  pane_id: string;
  command: string;
  path: string;
  width: number;
  height: number;
  title: string;
  agent: string | null;
  kind: "agent" | "service" | "terminal";
  state: { state: "working" | "needs-you" | "done"; message: string; at: number } | null;
};

/** Where a pane sits in the sidebar: agents by state, then services, then shells. */
export function statusOf(p: Pane): string {
  if (p.kind !== "agent") return p.kind;
  return p.state?.state ?? "idle";
}

export type Snapshot = { history: string; text: string; cursor_x: number; cursor_y: number };

export type Output = { pane: string; data: number[] };

export const listPanes = () => invoke<Pane[]>("list_panes");
export const capture = (pane: string) => invoke<Snapshot>("capture", { pane });
export const attach = (session: string) => invoke<void>("attach", { session });
export const resize = (session: string, cols: number, rows: number) =>
  invoke<void>("resize", { session, cols, rows });
export const sendKeys = (session: string, pane: string, data: string) =>
  invoke<void>("send_keys", { session, pane, data });

/** The task name an agent put in the pane title, without its status glyph. */
export function taskOf(p: Pane): string {
  const t = p.title.replace(/^[^\p{L}\p{N}]+/u, "").replace(/\s+\|\s+\S+$/, "");
  return t || p.window_name;
}
