#!/usr/bin/env node
// Follows the app's JSON-lines log and prints errors as they happen plus a
// summary every 10 s: spawns a minute, the slowest commands, pane opens and
// stream rates. Checks the numbers against docs/perf-plan.md.
//
//   node scripts/monitor.mjs [log]     default $BRIEFPANE_LOG or /tmp/briefpane/briefpane.jsonl
import { closeSync, existsSync, openSync, readSync, statSync } from "node:fs";

const LOG = process.argv[2] ?? process.env.BRIEFPANE_LOG ?? "/tmp/briefpane/briefpane.jsonl";
const EVERY_MS = 10_000;
const SLOW_CMD_MS = 50;
const MAX_SPAWNS_MIN = 25;
const MAX_OPEN_MS = 300;

let offset = existsSync(LOG) ? statSync(LOG).size : 0;
let partial = "";
let win = fresh();

function fresh() {
  return { spawns: 0, cmds: new Map(), opens: [], streams: new Map(), errors: 0, slowPolls: [] };
}

const stamp = () => new Date().toTimeString().slice(0, 8);

function take(e) {
  switch (e.kind) {
    case "start":
      console.log(`${stamp()} START ${JSON.stringify(e)}`);
      break;
    case "spawn":
      win.spawns++;
      break;
    case "cmd": {
      const c = win.cmds.get(e.name) ?? { n: 0, max: 0, sum: 0 };
      c.n++;
      c.sum += e.ms;
      c.max = Math.max(c.max, e.ms);
      win.cmds.set(e.name, c);
      if (!e.ok) console.log(`${stamp()} CMD-ERR ${e.name}: ${e.err}`);
      break;
    }
    case "stream":
      win.streams.set(e.pane, e);
      break;
    case "perf":
      if (e.name === "pane-open") win.opens.push(e.ms);
      else win.slowPolls.push(`${e.name} ${Math.round(e.ms)}ms`);
      break;
    case "error":
      win.errors++;
      console.log(`${stamp()} ERROR [${e.src}] ${e.msg}${e.data ? ` ${JSON.stringify(e.data)}` : ""}`);
      break;
  }
}

function report() {
  const perMin = Math.round(win.spawns * (60_000 / EVERY_MS));
  const cmds = [...win.cmds.entries()]
    .sort((a, b) => b[1].max - a[1].max)
    .slice(0, 4)
    .map(([n, c]) => `${n} ${c.n}x slow, max ${c.max.toFixed(0)}ms`)
    .join(", ");
  const flags = [];
  if (perMin > MAX_SPAWNS_MIN) flags.push(`spawns ${perMin}/min > ${MAX_SPAWNS_MIN}`);
  const slow = [...win.cmds.entries()].filter(([, c]) => c.max > SLOW_CMD_MS).map(([n]) => n);
  if (slow.length) flags.push(`slow: ${slow.join(" ")}`);
  if (win.opens.some((ms) => ms > MAX_OPEN_MS)) flags.push(`pane open > ${MAX_OPEN_MS}ms`);
  const streams = [...win.streams.values()].map((s) => `${s.pane} ${(s.bytes_s / 1024).toFixed(1)}KB/s`).join(" ");
  console.log(
    `${stamp()} ${flags.length ? "WARN" : "ok  "} spawns ${perMin}/min | ${cmds || "no cmds"}` +
      (win.opens.length ? ` | opens ${win.opens.map((m) => Math.round(m)).join(",")}ms` : "") +
      (streams ? ` | ${streams}` : "") +
      (win.slowPolls.length ? ` | slow polls ${win.slowPolls.join(", ")}` : "") +
      (flags.length ? ` | ${flags.join("; ")}` : ""),
  );
  win = fresh();
}

function poll() {
  if (!existsSync(LOG)) return;
  const size = statSync(LOG).size;
  if (size < offset) offset = 0; // truncated or rotated
  if (size === offset) return;
  const buf = Buffer.alloc(size - offset);
  const fd = openSync(LOG, "r");
  readSync(fd, buf, 0, buf.length, offset);
  closeSync(fd);
  offset = size;
  const lines = (partial + buf.toString("utf8")).split("\n");
  partial = lines.pop() ?? "";
  for (const l of lines) {
    try {
      take(JSON.parse(l));
    } catch {
      console.log(`${stamp()} BADLINE ${l.slice(0, 120)}`);
    }
  }
}

console.log(`${stamp()} watching ${LOG}`);
setInterval(poll, 250);
setInterval(report, EVERY_MS);
