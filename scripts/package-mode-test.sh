#!/usr/bin/env bash
# Real package settings: cleanup-on release-balanced, rustc -Oz and MIR inlining.
# No arguments runs the complete fixture suite; names select a focused subset.
# Existing default-mode byte budgets are not applicable; sizes are still reported.
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"
export JS_EXTRA_ARGS='js-profile=release-balanced js-minify=on'
export JSC_RUST_OPT_LEVEL=z
export JSC_RUST_MIR_INLINE=on
unset JSC_SYSROOT_ARGS
if [ "${SKIP_BUILD:-0}" != 1 ]; then bash scripts/build.sh --locked; fi
export SKIP_BUILD=1
bash scripts/build_sysroot.sh
exec bash scripts/test.sh --require-core "$@"
