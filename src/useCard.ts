import { card } from "./tmux";
import { usePoll } from "./usePoll";

const REFRESH_MS = 3000;

/** The task card for the repo and branch at `path`, kept fresh. */
export function useCard(path: string | undefined) {
  const { value, reload } = usePoll("card", () => (path ? card(path) : Promise.resolve(null)), REFRESH_MS, [path]);
  return [value ?? null, reload] as const;
}
