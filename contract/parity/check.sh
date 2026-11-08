#!/bin/sh
# The parity harness, end to end.
#
#   ./check.sh            UPDATE: re-capture the SSR fixtures, then run parity
#   ./check.sh --check    CHECK:  fail if a capture would change, then run parity
#
# Same ergonomics as `harness/run-all.mjs --check` one directory up: without the
# flag the recorded inputs are regenerated, with it a difference is a failure.
# The difference in kind is where the recorded input comes from. The contract
# harness regenerates from pinned upstream sources, so its drift report is "did
# upstream change". This one regenerates from demo-app's own server, so its drift
# report is "did the server's response change" -- which after a rebuild it should,
# and that is the point.
#
# CHECK mode is the CI shape. It needs a built demo-app, and it does not build
# one: rebuilding another crate from here would race whoever is editing it and
# could capture SSR bytes from a different emitter than the islands.js on disk.

set -e

cd "$(dirname "$0")"

if [ ! -d node_modules ]; then
	echo "parity: node_modules is missing, installing jsdom"
	npm install --no-audit --no-fund
	echo ""
fi

echo "=== harness unit tests"
node test.mjs
echo ""

# The L2 gate: every corpus family's trace-parity verdict against the committed
# contract/fixtures/l2-status.json. It is not about hydration parity and it is not
# about demo-app, so it looks out of place here -- it is here because it costs a
# second, needs no build and no server, and the two contract-side entry points are
# this script and `harness/run-all.mjs --check`. Wiring it into both means neither
# can miss it. The natural home would be scripts/dom-test.sh, which deliberately
# does not fail on L2; see the script's own header for why that is still right and
# why this gate is the answer instead of changing it.
#
# It fails on an IMPROVEMENT too. Re-record with:
#   node --import ../harness/register-loader.mjs ../harness/check-l2-status.mjs --record
echo "=== L2 corpus trace parity"
node --import ../harness/register-loader.mjs ../harness/check-l2-status.mjs
echo ""

echo "=== SSR capture"
node capture-ssr.mjs "$@"
echo ""

echo "=== parity run"
node run.mjs
echo ""

# Same two modes as the SSR capture above and for the same reason: without the
# flag the goldens are re-recorded, with it a difference is a failure. It runs
# after the parity run because a .d.ts is a claim about a module the parity run
# has just finished proving things about, and before the budgets because a
# declaration file costs nothing to ship and has no budget.
#
# While the backend emits no .d.ts at all this reports PENDING and exits 0. It
# arms itself: the first recording makes a later absence a failure. See dts.mjs.
echo "=== .d.ts goldens"
node dts.mjs "$@"
echo ""

# After the run, not before: the run's own per-fixture budget checks are the ones
# that gate a fixture, and this is the whole picture -- every subject including the
# ones no single fixture owns (the page total, the loader) plus the coverage check
# that catches a module appearing in the delivery with no budget at all. Re-baseline
# with `node budgets.mjs --update`, and read the diff before keeping it.
echo "=== size budgets"
node budgets.mjs
