import React from "react";
import ReactDOM from "react-dom/client";
import "@xterm/xterm/css/xterm.css";
import "./styles.css";
import App from "./App";
import { log } from "./tmux";

window.addEventListener("error", (e) => {
  log("error", e.message || "uncaught error", {
    source: e.filename,
    line: e.lineno,
    col: e.colno,
    stack: e.error instanceof Error ? e.error.stack : undefined,
  });
});
window.addEventListener("unhandledrejection", (e) => {
  const r = e.reason;
  log("error", r instanceof Error ? r.message : String(r), {
    kind: "unhandledrejection",
    stack: r instanceof Error ? r.stack : undefined,
  });
});

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
