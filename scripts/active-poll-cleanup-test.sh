#!/usr/bin/env bash
# The active-call cleanup gate (CONTRACT.md). examples/active_poll_cleanup.rs throws from a host
# call while a Rust invocation is still on the stack; scripts/active-poll-cleanup-check.mjs runs 20
# rows (five cases, both Promise drivers, normally and with an injected exception) and requires each
# initialized local to drop, in order, before the Promise settles.
#
# The JIR lane runs under the release profile (js-profile=release-balanced, so js-cleanup=on) in
# each optimizer mode, off/report/on: 60 rows. off and report must emit identical JavaScript, and
# every source map must check. The baseline lane has no semantic cleanup lowering, so it must refuse
# this program rather than link one without its cleanup paths (cleanup-admission-test.sh checks the
# same refusal on a minimal fixture). Every result is retained. SKIP_BUILD=1 reuses the backend.
set -euo pipefail
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
# shellcheck source=integration-lock.sh
source "$script_dir/integration-lock.sh"
if [ "${SKIP_BUILD:-0}" != 1 ]; then "$script_dir/build.sh" >/dev/null; fi
mkdir -p "$root/build"
work=$(mktemp -d "$root/build/active-poll-cleanup.XXXXXX")
echo "active poll cleanup: retained output at $work"
(cd "$root" && cargo build -p js-extern-macro --locked) >"$work/macro.log" 2>&1
case "$(uname -s)" in
Darwin) macro="$root/target/debug/libjs_extern_macro.dylib" ;;
*) macro="$root/target/debug/libjs_extern_macro.so" ;;
esac
mkdir -p "$work/proc-macro"
ln -s "$macro" "$work/proc-macro/$(basename -- "$macro")"
node "$script_dir/make-esm-shim.mjs" "$work/shim.mjs"
compile_program() { # lane mode
  mkdir -p "$work/$1-$2"
  JS_EXTRA_ARGS="$JS_EXTRA_ARGS js-lane=$1 js-modules=esm js-shim-module=../shim.mjs js-source-map=on" \
    "$script_dir/compile_core.sh" "$root/examples/active_poll_cleanup.rs" "$work/$1-$2/program.mjs" \
    --extern "mires_js=$work/libmires_js.rlib" --search "$work/proc-macro" >"$work/$1-$2/program.mjs.compile.log" 2>&1
}
failures=0
for mode in off report on; do
  export JS_EXTRA_ARGS="js-profile=release-balanced js-opt=$mode"
  SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >"$work/sysroot-$mode.log" 2>&1
  "$script_dir/compile_core.sh" "$root/mires-js/src/lib.rs" "$work/libmires_js.rlib" \
    --crate-type rlib --crate-name mires_js --extern "js_extern_macro=$macro" >"$work/library-$mode.log" 2>&1
  compile_program jir "$mode"
  program="$work/jir-$mode/program.mjs"
  status=passed
  node "$script_dir/active-poll-cleanup-check.mjs" "$program" >"$program.result.json" 2>"$program.result.log" || status=FAILED
  rows=$(node -e 'const r = JSON.parse(require("fs").readFileSync(process.argv[1], "utf8")); console.log(r.rows.filter(row => row.passed).length + "/" + r.rows.length)' "$program.result.json" 2>/dev/null || echo "?/?")
  echo "jir/$mode: active poll cleanup $status ($rows rows)"
  [ "$status" = passed ] || failures=$((failures + 1))
  node "$script_dir/check-sourcemap.js" "$program" >"$program.map.log"
  if compile_program baseline "$mode"; then
    echo "baseline/$mode: compiled; expected a refusal to discard cleanup paths" >&2
    failures=$((failures + 1))
  elif grep -q 'requires semantic cleanup lowering; baseline fallback would discard its cleanup paths' \
    "$work/baseline-$mode/program.mjs.compile.log" && [ ! -e "$work/baseline-$mode/program.mjs" ]; then
    echo "baseline/$mode: refused as expected"
  else
    echo "baseline/$mode: failed without the cleanup refusal; see $work/baseline-$mode/program.mjs.compile.log" >&2
    failures=$((failures + 1))
  fi
done
cmp "$work/jir-off/program.mjs" "$work/jir-report/program.mjs"
test "$failures" = 0
