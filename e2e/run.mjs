// End-to-end check against the real app and a real tmux session: pick a
// pane in the sidebar, type into the composer, prove the text reached tmux
// and came back into the rendered pane. Saves a screenshot of the window.
//
// Needs: tauri-driver on PATH, WebKitWebDriver, and a binary from
// `pnpm tauri build --debug --no-bundle`.
import { execFileSync, spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
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
tmux("new-session", "-d", "-s", SESSION, "-n", "e2e", "-x", "100", "-y", "30", "bash --norc --noprofile");

const driver = spawn("tauri-driver", [], { stdio: "ignore" });
let browser;
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
    const text = await browser.execute(() => document.querySelector(".xterm-rows")?.textContent ?? "");
    return text.split(TOKEN).length > 2;
  });
  console.log("app: output rendered in the pane view");

  await browser.saveScreenshot(`${OUT}app.png`);
  console.log(`screenshot: ${OUT}app.png`);
} finally {
  await browser?.deleteSession().catch(() => {});
  driver.kill();
  try {
    tmux("kill-session", "-t", SESSION);
  } catch {}
}
