import { useCallback, useEffect, useState } from "react";
import { type Card, card } from "./tmux";

const REFRESH_MS = 3000;

/** The task card for the repo and branch at `path`, kept fresh. */
export function useCard(path: string | undefined) {
  const [value, setValue] = useState<Card | null>(null);
  const reload = useCallback(() => {
    if (!path) return setValue(null);
    card(path)
      .then(setValue)
      .catch(() => setValue(null));
  }, [path]);

  useEffect(() => {
    reload();
    const t = setInterval(reload, REFRESH_MS);
    return () => clearInterval(t);
  }, [reload]);

  return [value, reload] as const;
}
