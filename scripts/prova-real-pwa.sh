#!/usr/bin/env bash
#
# The prova real for the installable page: the built worker, against the built bundle.
#
# §0.1 moved the manifest and the service worker into phase 3 because Web Push on iOS
# needs a home-screen install, so the install path has to exist before the write path
# does. This is what says it works, or does not.
#
# The unit tests prove the worker's logic against a fake scope. They cannot see the
# build, and the build is where this breaks silently: the precache list is injected from
# a manifest written by a *different* Vite build, registration failure is deliberately
# swallowed, and a worker emitted as an ES module simply does not register in some
# browsers. Every one of those failures looks like success from inside vitest.
#
# What it asserts:
#
#   1. The built worker is a classic script and evaluates in a bare worker scope.
#   2. It wires install, activate and fetch on load — and nothing else.
#   3. Install precaches every file in its list, all of which exist in dist/.
#   4. Every script and stylesheet the shell loads is in that list. If the two builds
#      ever disagree, the offline page is blank and the worker reports success.
#   5. With the network cut, the page and its script are still served.
#   6. A request bound for the relay is not answered here (I2).
#   7. Activating drops the caches of older builds and keeps its own.
#   8. dist/ holds nothing but the deployable set, the manifest's start_url carries no
#      pairing material, the icons it names exist, and the worker embeds no credential.
#
# Usage: scripts/prova-real-pwa.sh

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$HERE"

echo "building the page and the worker…"
pnpm --filter @claudinio/web build >/dev/null

echo
node apps/web/prova-real/pwa.mjs
