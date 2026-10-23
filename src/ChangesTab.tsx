import type { FileContents } from "@pierre/diffs/react";
import { MultiFileDiff, WorkerPoolContext } from "@pierre/diffs/react";
import { getOrCreateWorkerPoolSingleton } from "@pierre/diffs/worker";
import DiffsWorker from "@pierre/diffs/worker/worker.js?worker";
import { useEffect, useMemo, useRef } from "react";
import { changes, type FileChange } from "./tmux";
import { usePoll } from "./usePoll";

const REFRESH_MS = 3000;
const DIFF_OPTIONS = {
  theme: { dark: "min-dark", light: "min-light" },
  themeType: "dark",
  diffStyle: "unified",
  overflow: "wrap",
} as const;

// Highlighting runs off the main thread. The pool lives for the whole app so
// its highlight cache survives the tab closing; it falls back to the main
// thread if the workers fail to start.
const pool = getOrCreateWorkerPoolSingleton({
  poolOptions: { workerFactory: () => new DiffsWorker(), poolSize: 2 },
  highlighterOptions: { theme: DIFF_OPTIONS.theme },
});

/** FNV-1a over the UTF-16 code units: a cache key, not a security boundary. */
function hash(s: string): string {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 0x01000193);
  return (h >>> 0).toString(36);
}

type Side = { old: FileContents; new: FileContents };

const side = (name: string, which: string, contents: string): FileContents => ({
  name,
  contents,
  cacheKey: `${which}:${name}:${contents.length}:${hash(contents)}`,
});

/**
 * One `{name, contents}` pair per file that keeps its identity while the
 * file's contents are unchanged, since the diff re-parses and re-highlights
 * whenever the objects it is given change.
 */
function useStableSides(files: FileChange[]): Map<string, Side> {
  const cache = useRef(new Map<string, Side>());
  return useMemo(() => {
    const next = new Map<string, Side>();
    for (const f of files) {
      const prev = cache.current.get(f.name);
      const oldText = f.old ?? "";
      const newText = f.new ?? "";
      next.set(f.name, {
        old: prev && prev.old.contents === oldText ? prev.old : side(f.name, "old", oldText),
        new: prev && prev.new.contents === newText ? prev.new : side(f.name, "new", newText),
      });
    }
    cache.current = next;
    return next;
  }, [files]);
}

const NO_FILES: FileChange[] = [];

/** The repo's uncommitted changes. Fetched only while this tab is shown. */
export default function ChangesTab({ path, onCount }: { path: string; onCount: (n: number | null) => void }) {
  const { value, error } = usePoll("changes", () => changes(path), REFRESH_MS, [path]);
  const files = value ?? NO_FILES;
  const sides = useStableSides(files);

  useEffect(() => onCount(value ? value.length : null), [value, onCount]);
  useEffect(() => () => onCount(null), [onCount]);

  return (
    <WorkerPoolContext.Provider value={pool}>
      {files.length ? (
        files.map((f) => {
          const s = sides.get(f.name);
          return (
            <div className="file" key={f.name} data-file={f.name}>
              {(f.old === null && f.new === null) || !s ? (
                <p className="faint mono">
                  {f.status} {f.name} · too large to show
                </p>
              ) : (
                <MultiFileDiff oldFile={s.old} newFile={s.new} options={DIFF_OPTIONS} />
              )}
            </div>
          );
        })
      ) : (
        <p className={error && !value ? "err" : "faint"}>
          {value ? "No uncommitted changes." : error ? `Could not read changes: ${error}` : "Loading…"}
        </p>
      )}
    </WorkerPoolContext.Provider>
  );
}
