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
};

export type Snapshot = { text: string; cursor_x: number; cursor_y: number };

export type Output = { pane: string; data: number[] };

export const listPanes = () => invoke<Pane[]>("list_panes");
export const capture = (pane: string) => invoke<Snapshot>("capture", { pane });
export const attach = (session: string) => invoke<void>("attach", { session });
export const sendKeys = (session: string, pane: string, data: string) =>
  invoke<void>("send_keys", { session, pane, data });

const AGENTS = ["claude", "codex"];

/** The agent a pane runs, judged by its foreground command. */
export function agentOf(p: Pane): string | null {
  const cmd = p.command.toLowerCase();
  return AGENTS.find((a) => cmd.includes(a)) ?? null;
}
