// Smoke check of the installed release build, which loads its pages through
// Tauri's own protocol rather than the dev server: a pane streams, the lazy
// Changes tab and its diff worker load, and the log has no errors.
//
// Needs: tauri-driver, WebKitWebDriver, and `scripts/install-desktop` run.
import { execFileSync, spawn } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";

const APP = process.env.BRIEFPANE_APP ?? `${homedir()}/.local/lib/briefpane/briefpane`;
const SESSION = "bp-release";
const tmux = (...args) => execFileSync("tmux", args, { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] });

async function waitFor(what, fn, ms = 15_000) {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    if (await fn()) return;
    await sleep(250);
  }
  throw new Error(`timed out waiting for ${what}`);
}

const REPO = mkdtempSync(`${tmpdir()}/bp-release-`);
const git = (...args) => execFileSync("git", ["-C", REPO, ...args], { stdio: "ignore" });
git("init", "-q");
writeFileSync(`${REPO}/hello.txt`, "one\ntwo\n");
git("add", ".");
git("-c", "user.name=e2e", "-c", "user.email=e2e@local", "commit", "-qm", "init");
writeFileSync(`${REPO}/hello.txt`, "one\nthree\n");
process.env.BRIEFPANE_LOG = `${REPO}.jsonl`;
process.env.BRIEFPANE_CARDS = `${REPO}-cards`;

tmux("new-session", "-d", "-s", SESSION, "-c", REPO, "bash --norc --noprofile");
const driver = spawn("tauri-driver", [], {
  stdio: ["ignore", "ignore", process.env.BRIEFPANE_DRIVER_ERR ? "inherit" : "ignore"],
});
let browser;
try {
  await sleep(1000);
  browser = await remote({
    hostname: "127.0.0.1",
    port: 4444,
    logLevel: "error",
    capabilities: { "tauri:options": { application: APP }, "wdio:enforceWebDriverClassic": true },
  });
  const pane = tmux("display", "-p", "-t", SESSION, "#{pane_id}").trim();
  const row = await browser.$(`button.task[data-pane="${pane}"]`);
  await row.waitForExist({ timeout: 15_000 });
  await row.click();

  await browser.waitUntil(async () => (await browser.$(`.paneview[data-ready="${pane}"]`)).isExisting(), {
    timeout: 15_000,
  });
  // WebKit stops painting an off-screen window and a release build has no
  // hook on the terminal, so streaming is proven by the app's own stream line,
  // which needs output lasting over a second.
  await (await browser.$(".xterm-helper-textarea")).addValue(
    "for i in $(seq 1 30); do seq 1 2000; sleep 0.05; done\uE007",
  );
  await waitFor("a stream line for the pane", () =>
    readFileSync(process.env.BRIEFPANE_LOG, "utf8")
      .split("\n")
      .some((l) => l.includes('"kind":"stream"') && l.includes(`"pane":"${pane}"`)),
  );
  console.log("release: pane output streams into the view");
  if (await browser.execute(() => "__bpTerm" in window)) throw new Error("the release build exposes test hooks");
  console.log("release: no test hooks");

  await (await browser.$("button=Changes")).click();
  await waitFor("the diff of hello.txt", async () =>
    (
      await browser.execute(
        () => document.querySelector('.file[data-file="hello.txt"] diffs-container')?.shadowRoot?.textContent ?? "",
      )
    ).includes("three"),
  );
  console.log("release: lazy Changes tab renders the diff");

  await sleep(1000);
  const errors = readFileSync(process.env.BRIEFPANE_LOG, "utf8")
    .split("\n")
    .filter(Boolean)
    .map((l) => JSON.parse(l))
    .filter((e) => e.kind === "error" || e.ok === false);
  if (errors.length) throw new Error(`errors in the log: ${JSON.stringify(errors)}`);
  console.log("release: no errors in the log");
} catch (e) {
  console.error(e);
  try {
    console.error("pane:", tmux("capture-pane", "-p", "-t", SESSION).trim().slice(-300));
    console.error(
      "view:",
      await browser.execute(() => document.querySelector(".xterm-rows")?.textContent?.slice(-300)),
    );
    console.error(
      "log:",
      readFileSync(process.env.BRIEFPANE_LOG, "utf8")
        .split("\n")
        .filter((l) => !l.includes('"spawn"'))
        .slice(-15)
        .join("\n"),
    );
  } catch {}
  process.exitCode = 1;
} finally {
  await Promise.race([browser?.deleteSession().catch(() => {}), sleep(5_000)]);
  driver.kill();
  try {
    tmux("kill-session", "-t", SESSION);
  } catch {}
  rmSync(REPO, { recursive: true, force: true });
  rmSync(`${REPO}-cards`, { recursive: true, force: true });
  if (!process.env.BRIEFPANE_KEEP_LOG) rmSync(`${REPO}.jsonl`, { force: true });
  process.exit();
}
