// End-to-end check against the real app and a real tmux session: pick a
// pane in the sidebar, type into the composer, prove the text reached tmux
// and came back into the rendered pane. Saves a screenshot of the window.
//
// Needs: tauri-driver on PATH, WebKitWebDriver, and a binary from
// `pnpm tauri build --debug --no-bundle`.
import { execFileSync, spawn } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
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

tmux("new-session", "-d", "-s", SESSION, "-n", "e2e", "-c", REPO, "-x", "100", "-y", "30", "bash --norc --noprofile");

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

  const input = await browser.$(".composer input");
  await input.waitForExist({ timeout: 5_000 });
  await input.setValue(`echo ${TOKEN}`);
  await browser.keys("Enter");

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

  // The view owns the size while in use: the pane takes the view's columns and rows.
  const view = await browser.execute(() => `${window.__bpTerm.cols}x${window.__bpTerm.rows}`);
  await waitFor("the pane to fit the view", () => tmux("display-message", "-p", "-t", SESSION, "#{pane_width}x#{pane_height}").trim() === view);
  console.log(`tmux: pane resized to ${tmux("display-message", "-p", "-t", SESSION, "#{pane_width}x#{pane_height}").trim()}`);

  // A pane running `claude` that its hook marked as waiting moves to Needs you.
  tmux("new-window", "-d", "-t", SESSION, "-n", "agent", "exec -a claude sleep 600");
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
  await row.click();
  await (await browser.$("button=Changes")).click();
  // The diff renders in a shadow root; read the edited line from there.
  await waitFor("the diff of hello.txt", async () =>
    (await browser.execute(() => {
      const host = document.querySelector('.file[data-file="hello.txt"] diffs-container');
      return host?.shadowRoot?.textContent ?? "";
    })).includes("three"),
  );
  console.log("app: uncommitted change listed in Changes");

  await browser.saveScreenshot(`${OUT}app.png`);
  console.log(`screenshot: ${OUT}app.png`);
} finally {
  await browser?.deleteSession().catch(() => {});
  driver.kill();
  if (stateFile) rmSync(stateFile, { force: true });
  rmSync(REPO, { recursive: true, force: true });
  try {
    tmux("kill-session", "-t", SESSION);
  } catch {}
}
