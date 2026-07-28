/// The browser half of the phase-3 prova real.
///
/// Runs the app's own modules — `pairing`, `wire`, `noise`, `session` — unmodified,
/// against a real relay and the real device stack. Node 22 has a global `WebSocket`,
/// `crypto.subtle` and X25519, so nothing here is a stand-in for the browser: it is
/// the same code with the same primitives.
///
/// That is the whole point. Everything else about this app is tested against itself,
/// which is exactly what missed the device attaching to the relay with no channel
/// token — nothing could ever have connected, and both suites stayed green.
///
/// Driven by `scripts/prova-real-remote.sh`. Prints machine-readable lines the script
/// checks; exits non-zero on anything it can decide by itself.

import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { argv, env, exit } from "node:process";

const dir = env.PROVA_DIR;
const expectRecords = Number(env.PROVA_RECORDS ?? "200");
if (!dir) {
  console.error("PROVA_DIR is required");
  exit(2);
}

// Loaded through Vite so the TypeScript and the `@claudinio/protocol` alias resolve the
// same way they do in the build. Importing the source directly would need a second
// toolchain and could diverge from what ships.
const { createServer } = await import("vite");
const server = await createServer({
  root: fileURLToPath(new URL("..", import.meta.url)),
  logLevel: "warn",
  server: { middlewareMode: true },
});

const { parsePairingCode, isStale } = await server.ssrLoadModule("/src/pairing.ts");
const { Session } = await server.ssrLoadModule("/src/session.ts");
const { findEdits } = await server.ssrLoadModule("/src/edits.ts");
const { diffLines } = await server.ssrLoadModule(
  "/../../packages/timeline-ui/src/diff.ts",
);

/// The one place this harness bends reality, and it bends it outside the code under
/// test.
///
/// The relay runs plain `ws://` on loopback for a local run, and the pairing parser
/// refuses `ws://` on purpose — a doctored code pointing at plain WebSocket leaks the
/// channel token to anyone on the path. That check is worth keeping strict, so the code
/// still says `wss://` and the *socket factory* downgrades it, for loopback only.
///
/// Refusing anything else matters: a harness that downgraded any host would be a
/// harness that could pass while the real thing talked in clear over the internet.
function loopbackSocket(url) {
  const parsed = new URL(url);
  const loopback = parsed.hostname === "127.0.0.1" || parsed.hostname === "localhost";
  if (parsed.protocol === "wss:" && loopback) parsed.protocol = "ws:";
  if (parsed.protocol !== "ws:" || !loopback) {
    throw new Error(`the prova real only dials loopback, not ${url}`);
  }

  const ws = new WebSocket(parsed.toString());
  ws.binaryType = "arraybuffer";
  const socket = {
    send: (data) => ws.send(data),
    close: () => ws.close(),
    onopen: null,
    onmessage: null,
    onclose: null,
    onerror: null,
  };
  ws.onopen = () => socket.onopen?.();
  ws.onmessage = (event) => socket.onmessage?.(new Uint8Array(event.data));
  ws.onclose = () => socket.onclose?.();
  ws.onerror = () => socket.onerror?.();
  return socket;
}

const url = argv[2];
if (!url) {
  console.error("usage: peer.mjs <pairing-url>");
  exit(2);
}

// Parsed by the real parser, from the URL the device printed. If the device ever emits
// a code this refuses, that is the finding.
const parsed = parsePairingCode(new URL(url).hash);
if (!parsed.ok) {
  console.log(`PEER_CODE_REFUSED=${JSON.stringify(parsed.error)}`);
  exit(1);
}
if (isStale(parsed.code)) {
  console.log("PEER_CODE_STALE=1");
  exit(1);
}
console.log("PEER_CODE_PARSED=1");

const records = [];
let sas = null;
let confirmed = false;
let closedReason = null;
let failed = null;

const session = new Session(parsed.code, {
  onState: (state) => {
    console.log(`PEER_STATE=${state.kind}`);
    if (state.kind === "confirming") {
      sas = state.sas;
      console.log(`PEER_SAS=${state.sas}`);
      // Compared against the device's, by the script, before anything is confirmed.
      // The words are the security boundary; a peer that confirmed its own pairing
      // would be the peer replacing the human the check exists for.
      writeFileSync(`${dir}/peer-sas`, state.sas);
    }
    if (state.kind === "closed") closedReason = state.reason;
    if (state.kind === "failed") failed = state.why;
  },
  onMessage: (message) => {
    if (message.kind === "snapshot" && Array.isArray(message.records)) {
      records.push(...message.records);
      console.log(`PEER_SNAPSHOT=${message.records.length} total=${records.length}`);
    } else if (message.kind === "event") {
      records.push(message.event);
    } else {
      console.log(`PEER_OTHER=${message.kind}`);
    }
  },
}, loopbackSocket);

session.start();
session.subscribe("prova-real", 0);

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/// Wait for the script to say the words matched, then confirm — the same order a
/// person does it in.
const deadline = Date.now() + 90_000;
let matched = false;
while (Date.now() < deadline) {
  if (!confirmed && sas) {
    try {
      const verdict = readFileSync(`${dir}/sas-verdict`, "utf8").trim();
      if (verdict === "match") {
        matched = true;
        confirmed = true;
        console.log("PEER_CONFIRMING=1");
        session.confirm();
      } else if (verdict === "mismatch") {
        console.log("PEER_REFUSING=1");
        session.stop();
        break;
      }
    } catch {
      // Not written yet.
    }
  }
  if (records.length >= expectRecords) break;
  if (closedReason || failed) break;
  await sleep(100);
}

console.log(`PEER_RECORDS=${records.length}`);
console.log(`PEER_EXPECTED=${expectRecords}`);
console.log(`PEER_CONFIRMED=${matched ? 1 : 0}`);
if (closedReason) console.log(`PEER_CLOSED=${closedReason}`);
if (failed) console.log(`PEER_FAILED=${failed}`);

// The diffs, computed in the browser from the device's transcript. The device never
// sent one — an `edit_file` call carries the before and after — so this is the whole
// path §7 depends on, exercised rather than assumed.
const diffs = records
  .flatMap((record) => findEdits(record))
  .map((edit) => ({ path: edit.path, ...diffLines(edit.oldText, edit.newText) }));
for (const diff of diffs) {
  console.log(`PEER_DIFF=${diff.path} +${diff.added} -${diff.removed} hunks=${diff.hunks.length}`);
}
console.log(`PEER_DIFFS=${diffs.length}`);

// The transcript, so the script can check it is the device's and not an artefact.
writeFileSync(`${dir}/peer-records.json`, JSON.stringify(records, null, 2));
writeFileSync(`${dir}/peer-diffs.json`, JSON.stringify(diffs, null, 2));

session.stop();
await server.close();

if (failed) exit(1);
if (!matched) {
  console.error("the words were never confirmed");
  exit(1);
}
if (records.length !== expectRecords) {
  console.error(`expected ${expectRecords} records, got ${records.length}`);
  exit(1);
}
console.log("PEER_DONE=1");
exit(0);
