import { type DependencyList, useCallback, useEffect, useRef, useState } from "react";
import { log } from "./tmux";

const SLOW_MS = 100;

export type Polled<T> = { value: T | undefined; error: string | null; reload: () => void };

/**
 * Runs `fn` every `ms` while the window is visible and again when it becomes
 * visible. Runs never overlap. State changes only when the result's JSON
 * changes, so an unchanged value keeps its identity and renders nothing.
 * Changing `deps` restarts the poll and clears the value.
 */
export function usePoll<T>(name: string, fn: () => Promise<T>, ms: number, deps: DependencyList): Polled<T> {
  const [state, setState] = useState<{ value: T | undefined; error: string | null }>({ value: undefined, error: null });
  const fnRef = useRef(fn);
  fnRef.current = fn;
  const runRef = useRef<() => void>(() => {});

  useEffect(() => {
    let live = true;
    let running = false;
    let again = false;
    let lastJson: string | undefined;
    let lastError: string | null = null;
    setState({ value: undefined, error: null });

    const run = async () => {
      if (!live) return;
      if (running) {
        again = true;
        return;
      }
      if (document.hidden) return;
      running = true;
      const t0 = performance.now();
      try {
        const value = await fnRef.current();
        const json = JSON.stringify(value) ?? "";
        if (live && (json !== lastJson || lastError !== null)) {
          lastJson = json;
          lastError = null;
          setState({ value, error: null });
        }
      } catch (e) {
        const error = String(e);
        if (live && error !== lastError) {
          lastError = error;
          log("error", `poll ${name} failed`, { error });
          setState((s) => ({ value: s.value, error }));
        }
      } finally {
        running = false;
        const took = performance.now() - t0;
        if (took > SLOW_MS) log("perf", `poll:${name}`, { ms: Math.round(took) });
      }
      if (again) {
        again = false;
        void run();
      }
    };
    runRef.current = run;

    void run();
    const timer = setInterval(run, ms);
    const onVisible = () => {
      if (!document.hidden) void run();
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      live = false;
      runRef.current = () => {};
      clearInterval(timer);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [ms, ...deps]);

  const reload = useCallback(() => runRef.current(), []);
  return { ...state, reload };
}
