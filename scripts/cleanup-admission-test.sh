#!/usr/bin/env bash
# A cleanup obligation must survive even when JIR encoding is disabled or declined.
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"
work="$root/build/cleanup-admission"
mkdir -p "$work"
export JS_EXTRA_ARGS='js-cleanup=on js-jir=off js-opt=off'
SKIP_BUILD=1 bash scripts/build_sysroot.sh >"$work/sysroot.log" 2>&1
bash scripts/compile_core.sh examples/cleanup-admission/library.rs "$work/libguardlib.rlib" --crate-name guardlib --crate-type rlib >"$work/library.log" 2>&1
bash scripts/compile_core.sh examples/cleanup-admission/dead.rs "$work/dead.js" --extern "guardlib=$work/libguardlib.rlib" >"$work/dead.log" 2>&1
if bash scripts/compile_core.sh examples/cleanup-admission/live.rs "$work/live.js" --extern "guardlib=$work/libguardlib.rlib" >"$work/live.log" 2>&1; then
  echo 'FAIL: reachable cleanup compiled without any semantic body' >&2; exit 1
fi
grep -q 'requires semantic cleanup lowering' "$work/live.log"
[ ! -f "$work/live.js" ]
cat runtime/shim.js "$work/dead.js" scripts/entry.js | node - >"$work/dead-output.log"
echo 'cleanup admission: live declined body refused; unreachable declined dependency executes'
