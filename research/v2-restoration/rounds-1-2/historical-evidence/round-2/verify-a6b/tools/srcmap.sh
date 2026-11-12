#!/usr/bin/env bash
# srcmap.sh: the refusals and an accepted static-entry split with `js-source-map=on` (and
# `js-comments`), compiled directly by the worktree's compiler: is every refusal still one clean
# `error:` line, with no panic, and does the accepted split still load in every file order?
set -uo pipefail
source "$(dirname "$0")/env.sh"
core=$W/examples/core-tests
export RCGJS_INTEGRATION_LOCK_HELD=$W/build/.integration-lock
one() { # one <name> <src> <args...>
  local name=$1 src=$2; shift 2
  local out=$V/out/srcmap/$name
  rm -rf "$out"; mkdir -p "$out"
  (cd "$W" && SKIP_BUILD=1 JS_EXTRA_ARGS="$*" bash scripts/compile_core.sh "$src" "$out/program.js") > "$out/compile.log" 2>&1
  local status=$?
  echo "$name: exit $status; errors: $(grep -c '^error' "$out/compile.log"); panics: $(grep -ci 'panicked\|internal error\|RUST_BACKTRACE' "$out/compile.log"); $(grep -m1 '^error: the\|^error: rustc_codegen_js: the' "$out/compile.log" | cut -c1-150)"
}
for extra in "js-source-map=on" "js-profile=release-balanced js-source-map=on" "js-comments=on js-line-comments=on"; do
  tag=$(printf '%s' "$extra" | tr -c 'a-z0-9' '_')
  one "266-s-$tag" "$core/266_chunk_static_entry_refusals.rs" $extra js-modules=esm js-chunk=s:ENTRY_S js-chunk=a:chunk_a js-chunk=b:chunk_b
  one "266-c-$tag" "$core/266_chunk_static_entry_refusals.rs" $extra js-modules=esm js-chunk=c:ENTRY_C js-chunk=a:chunk_a js-chunk=b:chunk_b
  one "268-$tag" "$core/268_static_cycle_proto_key.rs" $extra
  one "265-$tag" "$core/265_chunk_static_entries.rs" $extra js-modules=esm js-chunk=u:ENTRY_UP js-chunk=d:ENTRY_DOWN js-chunk=a:chunk_a js-chunk=b:chunk_b
  echo "   265 load orders: $(node "$V/tools/orders.mjs" "$V/out/srcmap/265-$tag" "$W/build/esm/shim.js" "$core/265_chunk_static_entries.expected" a:chunk_a,b:chunk_b | cut -c1-160)"
done
