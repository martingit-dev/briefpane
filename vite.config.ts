// @ts-expect-error type error without @types/node package
import process from "node:process";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(() => ({
  plugins: [react()],
  // Hooks the e2e reads (the terminal buffer, frame counts): in the dev server
  // and debug builds, never in a release build.
  define: { __BP_TEST_HOOKS__: JSON.stringify(process.env.TAURI_ENV_DEBUG !== "false") },
  // The diff highlighter's worker imports modules, which a classic worker cannot.
  worker: { format: "es" as const },
  // Pre-bundled at startup: discovered on first open of the lazy Changes tab,
  // the dev server re-optimises and reloads, failing that import.
  optimizeDeps: { include: ["@pierre/diffs/react", "@pierre/diffs/worker"] },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
