// End-to-end check against the real app and a real tmux session: pick a
// pane in the sidebar, type into its terminal, prove the text reached tmux
// and came back into the rendered pane. Saves a screenshot of the window.
//
// Needs: tauri-driver on PATH, WebKitWebDriver, and the debug binary from
// `pnpm tauri dev` (served by the dev server) or `pnpm tauri build --debug
// --no-bundle`; both carry the test hooks, a release build does not.
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
const cardCli = (args, input) => execFileSync("briefpane-card", [...args, "--cwd", REPO], { input, encoding: "utf8" });
const gate = () => {
  try {
    execFileSync("briefpane-card", ["gate"], {
      input: JSON.stringify({ cwd: REPO }),
      stdio: ["pipe", "ignore", "ignore"],
    });
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
// A stub xdg-open records what the app hands to the system browser.
writeFileSync(`${stub}/xdg-open`, `#!/bin/sh\necho "$1" >>"${stub}/opened"\n`, { mode: 0o755 });
const driver = spawn("tauri-driver", [], {
  stdio: "ignore",
  env: { ...process.env, PATH: `${stub}:${process.env.PATH}` },
});
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

  const row = await browser.$(`button.task[data-pane="${tmux("display", "-p", "-t", SESSION, "#{pane_id}").trim()}"]`);
  await row.waitForExist({ timeout: 10_000 });
  await row.click();

  // Type into the terminal itself, as you would.
  // Typing during the opening snapshot can paint a line twice (docs/perf-plan.md, known issues).
  const ready = (pane) =>
    browser.waitUntil(async () => (await browser.$(`.paneview[data-ready="${pane}"]`)).isExisting(), {
      timeout: 10_000,
    });
  await ready(tmux("display", "-p", "-t", SESSION, "#{pane_id}").trim());
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
  }).catch(async (e) => {
    const why = await browser.execute(() => ({
      frames: window.__bpFrames,
      rows: window.__bpTerm?.rows,
      text: (() => {
        const b = window.__bpTerm?.buffer.active;
        let out = "";
        for (let i = 0; b && i < b.length; i++) out += `${b.getLine(i)?.translateToString(true) ?? ""}\n`;
        return out.trim().slice(-300);
      })(),
    }));
    throw new Error(`${e.message}: ${JSON.stringify(why)}`);
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
  await waitFor(
    "Ctrl+C to end cat",
    () => tmux("display-message", "-p", "-t", SESSION, "#{pane_current_command}").trim() === "bash",
  );
  console.log("tmux: arrows, Shift+Tab, Esc and Ctrl+C arrive as a terminal sends them");

  // The view owns the size while in use: the pane takes the view's columns and rows.
  // Read both sides each time: the view follows tmux and can settle a step later.
  const viewSize = () => browser.execute(() => `${window.__bpTerm.cols}x${window.__bpTerm.rows}`);
  const paneSize = () => tmux("display-message", "-p", "-t", SESSION, "#{pane_width}x#{pane_height}").trim();
  await waitFor("the pane to fit the view", async () => paneSize() === (await viewSize())).catch(async (e) => {
    throw new Error(`${e.message}: view ${await viewSize()}, pane ${paneSize()}`);
  });
  console.log(
    `tmux: pane resized to ${tmux("display-message", "-p", "-t", SESSION, "#{pane_width}x#{pane_height}").trim()}`,
  );

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
  execFileSync(
    "briefpane-hook",
    ["codex", "done", JSON.stringify({ "last-assistant-message": "## All green\n\nRan `pnpm e2e`." })],
    {
      env: { ...process.env, TMUX_PANE: agentPane },
    },
  );
  await waitFor("the agent row", async () => (await browser.$$("button.task")).length > 0);
  await browser.execute((pane) => {
    for (const b of document.querySelectorAll("button.task")) if (b.dataset.pane === pane) b.click();
  }, agentPane);
  await waitFor(
    "the brief to render",
    async () => (await browser.execute(() => document.querySelector(".md h2")?.textContent)) === "All green",
  );
  console.log("app: agent's answer rendered as markdown");

  // Back on the scratch repo's pane, Changes shows the edited file.
  tmux("send-keys", "-t", SESSION, "clear; printf 'top\\nmiddle\\n'", "Enter");
  await sleep(300);
  await row.click();

  // Reopening a pane whose screen ends in blank rows puts the cursor where
  // tmux has it, not shifted by the blank rows.
  await waitFor("the reopened view to match tmux's cursor", async () => {
    const [x, y] = tmux("display-message", "-p", "-t", SESSION, "#{cursor_x} #{cursor_y}")
      .trim()
      .split(" ")
      .map(Number);
    const tmuxLine = tmux("capture-pane", "-p", "-t", SESSION).split("\n")[y].trimEnd();
    const view = await browser.execute(() => {
      const b = window.__bpTerm?.buffer.active;
      return (
        b && {
          x: b.cursorX,
          y: b.cursorY,
          line: b
            .getLine(b.viewportY + b.cursorY)
            ?.translateToString(true)
            .trimEnd(),
        }
      );
    });
    return view && view.x === x && view.y === y && view.line === tmuxLine;
  });
  console.log("app: reopened pane has the cursor on tmux's row and column");

  // A pane in a split gets only part of the window. The view must draw the
  // pane's own size, or output lands on the wrong rows.
  tmux("split-window", "-d", "-h", "-t", `${SESSION}:e2e`, "bash --norc --noprofile");
  await waitFor("the view to follow the split pane's size", async () => {
    const [w, h] = tmux("display-message", "-p", "-t", `${SESSION}:e2e.0`, "#{pane_width} #{pane_height}")
      .trim()
      .split(" ")
      .map(Number);
    const v = await browser.execute(() => [window.__bpTerm.cols, window.__bpTerm.rows]);
    return v[0] === w && v[1] === h;
  });
  console.log(
    `app: view follows the split pane at ${tmux("display-message", "-p", "-t", `${SESSION}:e2e.0`, "#{pane_width}x#{pane_height}").trim()}`,
  );
  await (await browser.$("button=Changes")).click();
  // The diff renders in a shadow root; read the edited line from there.
  await waitFor("the diff of hello.txt", async () =>
    (
      await browser.execute(() => {
        const host = document.querySelector('.file[data-file="hello.txt"] diffs-container');
        return host?.shadowRoot?.textContent ?? "";
      })
    ).includes("three"),
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
  await waitFor(
    "the phase strip to show build",
    async () => (await browser.execute(() => document.querySelector(".phase .on")?.textContent)) === "build",
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
  await waitFor(
    "the app to select the new terminal",
    async () => (await browser.execute(() => document.querySelector("button.task.on")?.dataset.pane)) === newPane,
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
      return (
        args.includes("--disallowedTools") && args.includes("Edit,Write") && args.includes("briefpane-card finding")
      );
    } catch {
      return false;
    }
  });
  // The reviewer window opens before the card is updated.
  await waitFor("review to set phase and builder", () => {
    const m = JSON.parse(cardCli(["json"])).meta;
    return m.phase === "review" && m.builder === agentPane;
  });
  console.log("app: review started as a fresh read-only claude");

  cardCli(["finding", "P1", "Nightly backup lacks the wait"]);
  cardCli(["finding", "P2", "Cap is not configurable"]);
  const verdict = (id, v) =>
    browser.execute(
      (id, v) => {
        for (const f of document.querySelectorAll(".finding"))
          if (f.textContent.includes(id))
            for (const b of f.querySelectorAll("button")) if (b.textContent === v) b.click();
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

  // A session killed and recreated under the same name streams again.
  const AGAIN = `${SESSION}-again`;
  const termText = () =>
    browser.execute(() => {
      const b = window.__bpTerm?.buffer.active;
      let out = "";
      for (let i = 0; b && i < b.length; i++) out += `${b.getLine(i)?.translateToString(true) ?? ""}\n`;
      return out;
    });
  const openAgain = async (token) => {
    const id = tmux("new-session", "-d", "-P", "-F", "#{pane_id}", "-s", AGAIN, "bash --norc --noprofile").trim();
    const r = await browser.$(`button.task[data-pane="${id}"]`);
    await r.waitForExist({ timeout: 10_000 });
    await r.click();
    await waitFor(
      "the recreated pane to be selected",
      async () => (await browser.execute(() => document.querySelector("button.task.on")?.dataset.pane)) === id,
    );
    await ready(id);
    await (await browser.$(".xterm-helper-textarea")).addValue(`echo ${token}\uE007`);
    await waitFor(`${token} streamed into the view`, async () => (await termText()).split(token).length > 2);
  };
  await openAgain(`${TOKEN}-a`);
  tmux("kill-session", "-t", AGAIN);
  await sleep(1000);
  await openAgain(`${TOKEN}-b`);
  tmux("kill-session", "-t", AGAIN);
  console.log("app: a recreated session streams again");

  // Detaching the app's client (as `tmux attach -d` does) reopens the open view.
  const live = tmux(
    "new-window",
    "-d",
    "-P",
    "-F",
    "#{pane_id}",
    "-t",
    `${SESSION}:`,
    "bash --norc --noprofile",
  ).trim();
  await (await browser.$(`button.task[data-pane="${live}"]`)).click();
  await ready(live);
  tmux("detach-client", "-s", SESSION);
  // The old view's marker goes first; otherwise the stale one would pass the check below.
  await waitFor("the view to close after the detach", async () =>
    browser.execute(() => !document.querySelector(".paneview")?.dataset.ready),
  );
  await waitFor("the view to reopen after the detach", async () =>
    browser.execute((p) => {
      const v = document.querySelector(".paneview");
      return v?.dataset.ready === p && !v.dataset.error;
    }, live),
  );
  await (await browser.$(".xterm-helper-textarea")).addValue(`echo ${TOKEN}-d\uE007`);
  await waitFor("output after the reopen", async () => (await termText()).split(`${TOKEN}-d`).length > 2);
  console.log("app: a detached view reopens and streams");

  // A clicked link opens in the browser and the window stays on the app.
  const home = await browser.execute(() => location.href);
  await browser.execute(() => {
    const a = document.createElement("a");
    a.href = "https://example.com/bp-e2e";
    a.textContent = "link";
    document.body.append(a);
    a.click();
  });
  await waitFor("the link to reach the browser", () => {
    try {
      return readFileSync(`${stub}/opened`, "utf8").includes("https://example.com/bp-e2e");
    } catch {
      return false;
    }
  });
  if ((await browser.execute(() => location.href)) !== home) throw new Error("the window left the app");
  console.log("app: a link opens in the browser and the app stays put");

  // Typing in another terminal takes the window's size; typing in the app takes it back.
  const sizePane = tmux(
    "new-window",
    "-d",
    "-P",
    "-F",
    "#{pane_id}",
    "-t",
    `${SESSION}:`,
    "bash --norc --noprofile",
  ).trim();
  const widthOf = () => Number(tmux("display", "-p", "-t", sizePane, "#{window_width}").trim());
  const sizeRow = await browser.$(`button.task[data-pane="${sizePane}"]`);
  await sizeRow.waitForExist({ timeout: 10_000 });
  await sizeRow.click();
  await sleep(1500);
  const appWidth = widthOf();
  const other = spawn("script", ["-qfc", `stty cols 200 rows 50; tmux attach -t ${SESSION}`, "/dev/null"], {
    stdio: ["pipe", "ignore", "ignore"],
  });
  try {
    await sleep(1000);
    tmux("select-window", "-t", sizePane);
    other.stdin.write(" ");
    await waitFor("the other terminal to take the size", () => widthOf() === 200);
    await sleep(1100);
    await (await browser.$(".xterm-helper-textarea")).addValue(" ");
    await waitFor(`the app to take the size back from 200 to ${appWidth}`, () => widthOf() === appWidth);
  } finally {
    other.kill();
  }
  console.log(`app: typing in the app takes the size back (200 to ${appWidth} cols)`);

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
  // tauri-driver can exit before the app it started; never leave one behind.
  try {
    execFileSync("pkill", ["-f", `^${APP}$`]);
  } catch {}
  if (stateFile) rmSync(stateFile, { force: true });
  rmSync(REPO, { recursive: true, force: true });
  rmSync(`${REPO}-cards`, { recursive: true, force: true });
  rmSync(stub, { recursive: true, force: true });
  try {
    tmux("kill-session", "-t", `${SESSION}-again`);
  } catch {}
  try {
    tmux("kill-session", "-t", SESSION);
  } catch {}
  // A lingering WebDriver socket would keep node alive.
  process.exit(process.exitCode ?? 0);
}
