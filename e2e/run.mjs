// End-to-end check against the real app and a real tmux session: pick a
// pane in the sidebar, type into its terminal, prove the text reached tmux
// and came back into the rendered pane. Saves a screenshot of the window.
//
// Needs: tauri-driver on PATH, WebKitWebDriver, and a binary from
// `pnpm tauri build --debug --no-bundle`.
import { execFileSync, spawn } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";

const APP = new URL("../src-tauri/target/debug/briefpane", import.meta.url).pathname;
const OUT = new URL("./out/", import.meta.url).pathname;
const SESSION = "bp-e2e";
const TOKEN = `bp-ok-${process.pid}`;

const tmux = (...args) => execFileSync("tmux", args, { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] });

async function waitFor(what, fn, ms = 10_000) {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    if (await fn()) return;
    await sleep(250);
  }
  throw new Error(`timed out waiting for ${what}`);
}

mkdirSync(OUT, { recursive: true });
try {
  tmux("kill-session", "-t", SESSION);
} catch {}
// A scratch repo with one committed file and one edit, so Changes has a known diff.
const REPO = mkdtempSync(`${tmpdir()}/bp-e2e-`);
const git = (...args) => execFileSync("git", ["-C", REPO, ...args], { stdio: "ignore" });
git("init", "-q");
writeFileSync(`${REPO}/hello.txt`, "one\ntwo\n");
git("add", ".");
git("-c", "user.name=e2e", "-c", "user.email=e2e@local", "commit", "-qm", "init");
writeFileSync(`${REPO}/hello.txt`, "one\nthree\n");

// Cards for this run go to a scratch folder; the app inherits this through tauri-driver.
process.env.BRIEFPANE_CARDS = `${REPO}-cards`;
const cardCli = (args, input) =>
  execFileSync("briefpane-card", [...args, "--cwd", REPO], { input, encoding: "utf8" });
const gate = () => {
  try {
    execFileSync("briefpane-card", ["gate"], { input: JSON.stringify({ cwd: REPO }), stdio: ["pipe", "ignore", "ignore"] });
    return 0;
  } catch (e) {
    return e.status;
  }
};
cardCli(["define", "When I open the repo, I see hello.txt changed"]);
cardCli(["plan"], "1. Edit hello.txt\n2. Prove it in Changes");
cardCli(["proof", "Changes lists hello.txt"]);

tmux("new-session", "-d", "-s", SESSION, "-n", "e2e", "-c", REPO, "-x", "100", "-y", "30", "bash --norc --noprofile");

const stub = mkdtempSync(`${tmpdir()}/bp-stub-`);
process.env.BRIEFPANE_CLAUDE = `${stub}/claude`;
const driver = spawn("tauri-driver", [], { stdio: "ignore" });
let browser;
let stateFile;
try {
  await sleep(1000);
  browser = await remote({
    hostname: "127.0.0.1",
    port: 4444,
    logLevel: "error",
    // Classic WebDriver: WebKitWebDriver has no BiDi.
    capabilities: { "tauri:options": { application: APP }, "wdio:enforceWebDriverClassic": true },
  });

  const row = await browser.$(`button.task*=${SESSION}`);
  await row.waitForExist({ timeout: 10_000 });
  await row.click();

  // Type into the terminal itself, as you would.
  const textarea = await browser.$(".xterm-helper-textarea");
  await textarea.waitForExist({ timeout: 10_000 });
  await textarea.addValue(`echo ${TOKEN}\uE007`);

  // The command line plus its output: two occurrences in the real pane.
  await waitFor("tmux to run the command", () => tmux("capture-pane", "-p", "-t", SESSION).split(TOKEN).length > 2);
  console.log("tmux: command ran in the pane");

  await waitFor("the output to render in the app", async () => {
    // The terminal buffer, not the DOM: WebKit stops painting an off-screen window.
    const text = await browser.execute(() => {
      const b = window.__bpTerm?.buffer.active;
      if (!b) return "";
      let out = "";
      for (let i = 0; i < b.length; i++) out += `${b.getLine(i)?.translateToString(true) ?? ""}\n`;
      return out;
    });
    return text.split(TOKEN).length > 2;
  });
  console.log("app: output rendered in the pane view");

  // Keys a TUI relies on reach the pane as the same bytes a terminal sends.
  // `cat -v` echoes them visibly: arrows, Shift+Tab, Esc, Ctrl+C.
  await textarea.addValue("cat -v\uE007");
  await sleep(300);
  await browser.keys(["ArrowUp", "ArrowDown"]);
  await browser.keys(["Shift", "Tab", "Shift"]);
  await browser.keys(["Escape", "Enter"]);
  await waitFor("special keys in the pane", () => {
    const shown = tmux("capture-pane", "-p", "-t", SESSION);
    return ["^[[A", "^[[B", "^[[Z", "^["].every((k) => shown.includes(k));
  });
  await browser.keys(["Control", "c", "Control"]);
  await waitFor("Ctrl+C to end cat", () => tmux("display-message", "-p", "-t", SESSION, "#{pane_current_command}").trim() === "bash");
  console.log("tmux: arrows, Shift+Tab, Esc and Ctrl+C arrive as a terminal sends them");

  // The view owns the size while in use: the pane takes the view's columns and rows.
  const view = await browser.execute(() => `${window.__bpTerm.cols}x${window.__bpTerm.rows}`);
  await waitFor("the pane to fit the view", () => tmux("display-message", "-p", "-t", SESSION, "#{pane_width}x#{pane_height}").trim() === view);
  console.log(`tmux: pane resized to ${tmux("display-message", "-p", "-t", SESSION, "#{pane_width}x#{pane_height}").trim()}`);

  // A pane running `claude` that its hook marked as waiting moves to Needs you.
  tmux("new-window", "-d", "-t", SESSION, "-n", "agent", "-c", REPO, "exec -a claude sleep 600");
  const agentPane = tmux("display-message", "-p", "-t", `${SESSION}:agent`, "#{pane_id}").trim();
  const ask = `Claude needs your permission ${process.pid}`;
  execFileSync("briefpane-hook", ["claude", "needs-you"], {
    input: JSON.stringify({ message: ask }),
    env: { ...process.env, TMUX_PANE: agentPane },
  });
  await waitFor("the agent under Needs you", async () => {
    const groups = await browser.execute(() =>
      [...document.querySelectorAll(".side section")].map((s) => s.textContent ?? ""),
    );
    return groups.some((g) => g.startsWith("Needs you") && g.includes(ask));
  });
  console.log("app: waiting agent listed under Needs you");
  stateFile = `${process.env.HOME}/.local/state/briefpane/panes/${agentPane.slice(1)}.json`;

  // Its answer renders as markdown in the Brief tab.
  execFileSync("briefpane-hook", ["codex", "done", JSON.stringify({ "last-assistant-message": "## All green\n\nRan `pnpm e2e`." })], {
    env: { ...process.env, TMUX_PANE: agentPane },
  });
  await waitFor("the agent row", async () => (await browser.$$("button.task")).length > 0);
  await browser.execute((pane) => {
    for (const b of document.querySelectorAll("button.task")) if (b.dataset.pane === pane) b.click();
  }, agentPane);
  await waitFor("the brief to render", async () =>
    (await browser.execute(() => document.querySelector(".md h2")?.textContent)) === "All green",
  );
  console.log("app: agent's answer rendered as markdown");

  // Back on the scratch repo's pane, Changes shows the edited file.
  tmux("send-keys", "-t", SESSION, "clear; printf 'top\\nmiddle\\n'", "Enter");
  await sleep(300);
  await row.click();

  // Reopening a pane whose screen ends in blank rows puts the cursor where
  // tmux has it, not shifted by the blank rows.
  await waitFor("the reopened view to match tmux's cursor", async () => {
    const [x, y] = tmux("display-message", "-p", "-t", SESSION, "#{cursor_x} #{cursor_y}").trim().split(" ").map(Number);
    const tmuxLine = tmux("capture-pane", "-p", "-t", SESSION).split("\n")[y].trimEnd();
    const view = await browser.execute(() => {
      const b = window.__bpTerm?.buffer.active;
      return b && { x: b.cursorX, y: b.cursorY, line: b.getLine(b.viewportY + b.cursorY)?.translateToString(true).trimEnd() };
    });
    return view && view.x === x && view.y === y && view.line === tmuxLine;
  });
  console.log("app: reopened pane has the cursor on tmux's row and column");

  // A pane in a split gets only part of the window. The view must draw the
  // pane's own size, or output lands on the wrong rows.
  tmux("split-window", "-d", "-h", "-t", `${SESSION}:e2e`, "bash --norc --noprofile");
  await waitFor("the view to follow the split pane's size", async () => {
    const [w, h] = tmux("display-message", "-p", "-t", `${SESSION}:e2e.0`, "#{pane_width} #{pane_height}").trim().split(" ").map(Number);
    const v = await browser.execute(() => [window.__bpTerm.cols, window.__bpTerm.rows]);
    return v[0] === w && v[1] === h;
  });
  console.log(`app: view follows the split pane at ${tmux("display-message", "-p", "-t", `${SESSION}:e2e.0`, "#{pane_width}x#{pane_height}").trim()}`);
  await (await browser.$("button=Changes")).click();
  // The diff renders in a shadow root; read the edited line from there.
  await waitFor("the diff of hello.txt", async () =>
    (await browser.execute(() => {
      const host = document.querySelector('.file[data-file="hello.txt"] diffs-container');
      return host?.shadowRoot?.textContent ?? "";
    })).includes("three"),
  );
  console.log("app: uncommitted change listed in Changes");

  // The plan gate: edits are refused until the human approves in the Card tab.
  if (gate() !== 2) throw new Error("gate should refuse edits before approval");
  await (await browser.$("button=Card")).click();
  const approve = await browser.$("button=Approve plan");
  await approve.waitForExist({ timeout: 10_000 });
  await approve.click();
  await waitFor("the plan to be approved", () => JSON.parse(cardCli(["json"])).meta.plan_approved !== "");
  if (gate() !== 0) throw new Error("gate should allow edits after approval");
  await waitFor("the phase strip to show build", async () =>
    (await browser.execute(() => document.querySelector(".phase .on")?.textContent)) === "build",
  );
  console.log("app: approving the plan lifted the edit gate");

  // New terminal opens in the selected pane's repo and the app selects it.
  const paneIds = () => tmux("list-panes", "-s", "-t", SESSION, "-F", "#{pane_id}").trim().split("\n");
  const before = new Set(paneIds());
  const windows = () => tmux("list-windows", "-t", SESSION).trim().split("\n").length;
  const windowsBefore = windows();
  await (await browser.$('button[title="New terminal"]')).click();
  let newPane;
  await waitFor("a new window in the repo", () => {
    if (windows() !== windowsBefore + 1) return false;
    const fresh = tmux("list-panes", "-s", "-t", SESSION, "-F", "#{pane_id}\t#{pane_current_path}")
      .trim()
      .split("\n")
      .map((l) => l.split("\t"))
      .find(([id]) => !before.has(id));
    newPane = fresh?.[0];
    return fresh?.[1] === REPO;
  });
  await waitFor("the app to select the new terminal", async () =>
    (await browser.execute(() => document.querySelector("button.task.on")?.dataset.pane)) === newPane,
  );
  console.log("app: new terminal opened in the selected pane's repo");

  // Review: a fresh agent starts read-only, findings get verdicts, fixes go
  // back to the building agent. A stub `claude` (BRIEFPANE_CLAUDE, set before
  // the app starts) records how it was launched instead of starting a real one.
  writeFileSync(`${stub}/claude`, `#!/bin/sh\nprintf '%s\\n' "$@" > ${stub}/args\nexec sleep 600\n`, { mode: 0o755 });

  await browser.execute((p) => {
    for (const b of document.querySelectorAll("button.task")) if (b.dataset.pane === p) b.click();
  }, agentPane);
  await (await browser.$("button=Card")).click();
  const startReview = await browser.$("button=Start review");
  await startReview.waitForExist({ timeout: 10_000 });
  await startReview.click();
  await waitFor("the reviewer to start read-only", () => {
    try {
      const args = readFileSync(`${stub}/args`, "utf8");
      return args.includes("--disallowedTools") && args.includes("Edit,Write") && args.includes("briefpane-card finding");
    } catch {
      return false;
    }
  });
  const afterStart = JSON.parse(cardCli(["json"]));
  if (afterStart.meta.phase !== "review" || afterStart.meta.builder !== agentPane) throw new Error("review should set phase and builder");
  console.log("app: review started as a fresh read-only claude");

  cardCli(["finding", "P1", "Nightly backup lacks the wait"]);
  cardCli(["finding", "P2", "Cap is not configurable"]);
  const verdict = (id, v) =>
    browser.execute(
      (id, v) => {
        for (const f of document.querySelectorAll(".finding"))
          if (f.textContent.includes(id)) for (const b of f.querySelectorAll("button")) if (b.textContent === v) b.click();
      },
      id,
      v,
    );
  await waitFor("the findings in the Card tab", async () => (await browser.$$(".finding")).length === 2);
  await verdict("R1", "fix");
  await verdict("R2", "reject");
  await waitFor("the verdicts on the card", () => {
    const r = JSON.parse(cardCli(["json"])).review;
    return r[0].state === "fix" && r[1].state === "reject";
  });
  const send = await browser.$("button=Send 1 fix to the agent");
  await send.waitForExist({ timeout: 10_000 });
  await send.click();
  await waitFor("the fix to reach the building agent", () =>
    tmux("capture-pane", "-p", "-J", "-t", agentPane).includes("R1 (P1) Nightly backup lacks the wait"),
  );
  console.log("app: verdicts saved and the fix sent to the building agent");

  // Best effort: WebKit waits for a paint that never comes off screen.
  const shot = await Promise.race([
    browser.saveScreenshot(`${OUT}app.png`).then(() => `screenshot: ${OUT}app.png`),
    sleep(10_000).then(() => "screenshot: skipped, the window is off screen"),
  ]);
  console.log(shot);
} catch (e) {
  console.error(e);
  process.exitCode = 1;
} finally {
  await Promise.race([browser?.deleteSession().catch(() => {}), sleep(5_000)]);
  driver.kill();
  if (stateFile) rmSync(stateFile, { force: true });
  rmSync(REPO, { recursive: true, force: true });
  rmSync(`${REPO}-cards`, { recursive: true, force: true });
  rmSync(stub, { recursive: true, force: true });
  try {
    tmux("kill-session", "-t", SESSION);
  } catch {}
  // A lingering WebDriver socket would keep node alive.
  process.exit(process.exitCode ?? 0);
}
