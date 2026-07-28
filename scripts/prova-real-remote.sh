#!/usr/bin/env bash
#
# The phase-3 prova real: a real browser peer, a real relay, and the real device stack.
#
# Everything about remote access is tested in units, and units are what missed the bug
# that mattered — the device attached to the relay without a channel token, so nothing
# could ever have connected while both suites stayed green. Two things each correct
# alone are not a working system, and this is the script that says so or does not.
#
# What it asserts:
#
#   1. The device and the browser derive the SAME THREE WORDS. Not that each derives
#      three — that they agree, which is the whole security value of the check.
#   2. Nothing is served before the words are confirmed.
#   3. Every transcript record reaches the browser.
#   4. The pairing was written to the device's book, by the real admit path.
#   5. A read-only peer never reaches the app through DeviceActions.
#   6. The browser computes a diff from a transcript that contains no diff.
#   7. No transcript text appears in the relay's log.
#
# Usage: scripts/prova-real-remote.sh [records]
#
# Needs the relay checked out next door. Set RELAY_REPO to point elsewhere.

set -euo pipefail

RECORDS="${1:-200}"
RELAY_REPO="${RELAY_REPO:-$HOME/claudinio_relay}"
PORT="${PORT:-8795}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

WORK="$(mktemp -d)"
trap 'cleanup' EXIT

RELAY_PID=""
cleanup() {
  [ -n "$RELAY_PID" ] && kill "$RELAY_PID" 2>/dev/null || true
  echo "logs in $WORK"
}

fail() { echo "FAIL  $*" >&2; exit 1; }
pass() { echo "PASS  $*"; }

# ── the relay ───────────────────────────────────────────────────────────────
#
# Plain ws:// on localhost. The pairing parser refuses ws:// on purpose, so the peer is
# handed a wss:// URL and Node is told to accept the loopback — the alternative is a
# self-signed certificate whose only effect would be to test TLS setup.
[ -d "$RELAY_REPO" ] || fail "no relay at $RELAY_REPO (set RELAY_REPO)"

echo "building the relay…"
(cd "$RELAY_REPO" && cargo build --quiet --bin claudinio-relay)
BIND="127.0.0.1:$PORT" RUST_LOG=info "$RELAY_REPO/target/debug/claudinio-relay" \
  >"$WORK/relay.log" 2>&1 &
RELAY_PID=$!

for _ in $(seq 1 50); do
  curl -sf "http://127.0.0.1:$PORT/healthz" >/dev/null 2>&1 && break
  sleep 0.2
done
curl -sf "http://127.0.0.1:$PORT/healthz" >/dev/null || fail "the relay did not come up"
echo "relay up on :$PORT"

# ── the device ──────────────────────────────────────────────────────────────
#
# The real transport, noise, bridge, dedup, policy, pairing and control. Only
# DeviceActions is a double, and nothing past that seam is on the wire.
echo "starting the real device stack…"
(
  cd "$HERE/src-tauri"
  RELAY_URL="ws://127.0.0.1:$PORT/ws" \
  PROVA_DIR="$WORK" \
  PROVA_RECORDS="$RECORDS" \
    cargo test --quiet --lib --features remote prova_real -- --ignored --nocapture
) >"$WORK/device.log" 2>&1 &
DEVICE_PID=$!

# The device prints its pairing URL once it has minted a code.
PAIRING_URL=""
for _ in $(seq 1 200); do
  PAIRING_URL="$(grep -m1 '^PROVA_URL=' "$WORK/device.log" 2>/dev/null | cut -d= -f2- || true)"
  [ -n "$PAIRING_URL" ] && break
  sleep 0.2
done
[ -n "$PAIRING_URL" ] || { cat "$WORK/device.log"; fail "the device never printed a pairing code"; }
echo "device is listening"

# The device dials plain ws:// on loopback; the browser's parser requires wss:// and
# should keep requiring it. So the code handed to the peer says wss://, and the peer's
# socket factory downgrades it for loopback only — the bend is in the harness, narrowly,
# rather than in the check.
PEER_URL="${PAIRING_URL//r=ws:\/\//r=wss:\/\/}"

# ── the peer ────────────────────────────────────────────────────────────────
echo "starting the browser peer…"
(
  cd "$HERE"
  PROVA_DIR="$WORK" PROVA_RECORDS="$RECORDS" \
  NODE_OPTIONS="--no-warnings" \
    node apps/web/prova-real/peer.mjs "$PEER_URL"
) >"$WORK/peer.log" 2>&1 &
PEER_PID=$!

# ── the word check ──────────────────────────────────────────────────────────
#
# The assertion that matters, and the reason neither side confirms itself: each derives
# the SAS from its own handshake, and only a genuinely end-to-end handshake makes them
# agree. The script is the human here.
DEVICE_SAS=""; PEER_SAS=""
for _ in $(seq 1 300); do
  DEVICE_SAS="$(grep -m1 '^PROVA_DEVICE_SAS=' "$WORK/device.log" 2>/dev/null | cut -d= -f2- || true)"
  [ -f "$WORK/peer-sas" ] && PEER_SAS="$(cat "$WORK/peer-sas")"
  [ -n "$DEVICE_SAS" ] && [ -n "$PEER_SAS" ] && break
  sleep 0.2
done

echo
echo "device words : ${DEVICE_SAS:-(none)}"
echo "browser words: ${PEER_SAS:-(none)}"
echo

[ -n "$DEVICE_SAS" ] || { cat "$WORK/device.log"; fail "the device never showed the words"; }
[ -n "$PEER_SAS" ] || { cat "$WORK/peer.log"; fail "the browser never showed the words"; }

if [ "$DEVICE_SAS" = "$PEER_SAS" ]; then
  pass "the words match on both sides"
  echo match >"$WORK/sas-verdict"
  touch "$WORK/confirmed"
else
  echo mismatch >"$WORK/sas-verdict"
  fail "the words differ — something sat between the two ends"
fi

# Nothing may have been served before that point.
if grep -q '^PEER_SNAPSHOT=' "$WORK/peer.log" 2>/dev/null; then
  fail "the browser received transcript before the words were confirmed"
fi
pass "nothing was served before the words were confirmed"

# ── let it run ──────────────────────────────────────────────────────────────
wait "$PEER_PID" && PEER_OK=1 || PEER_OK=0
touch "$WORK/peer-done"
wait "$DEVICE_PID" && DEVICE_OK=1 || DEVICE_OK=0

echo
sed -n 's/^PEER_/  peer  /p' "$WORK/peer.log" | tail -8
sed -n 's/^PROVA_DEVICE_/  device /p' "$WORK/device.log" | tail -8
echo

[ "$PEER_OK" = 1 ] || { tail -30 "$WORK/peer.log"; fail "the browser peer failed"; }
pass "every transcript record reached the browser ($RECORDS)"

# The transcript carries an edit_file call and no diff. Computing one in the browser is
# what §7's "a human reads the change before approving" rests on, so it is checked here
# rather than assumed from the unit tests.
grep -q '^PEER_DIFF=src/main.rs +2 -2 hunks=1' "$WORK/peer.log" \
  || { grep '^PEER_DIFF' "$WORK/peer.log" || true; fail "the browser did not compute the edit's diff"; }
pass "the browser computed a diff from the device's transcript"

grep -q '^PROVA_DEVICE_CONFIRMED=1' "$WORK/device.log" || fail "the device never saw a confirmation"
pass "the device served only after the confirmation"

grep -q '^PROVA_DEVICE_PAIRINGS=1' "$WORK/device.log" \
  || fail "the pairing was not written to the device's book"
pass "the pairing is in the device's book, written by the real admit path"

grep -q '^PROVA_DEVICE_ACTIONS_CALLED=0' "$WORK/device.log" \
  || fail "a read-only peer reached the app through DeviceActions"
pass "the read-only peer never reached the app"

[ "$DEVICE_OK" = 1 ] || { tail -30 "$WORK/device.log"; fail "the device harness failed"; }

# ── the relay stayed blind ──────────────────────────────────────────────────
if grep -qE 'record [0-9]+|prova-real"|assistant' "$WORK/relay.log"; then
  fail "transcript text appeared in the relay's log"
fi
pass "no transcript text in the relay's log"

echo
echo "PROVA REAL (phase 3): PASS"
