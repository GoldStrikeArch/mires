#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
fixture="$root/examples/core-tests/320_r3_loops_bounds.rs"
expected="$root/examples/core-tests/320_r3_loops_bounds.expected"

if [ "${SKIP_BUILD:-0}" != "1" ]; then
    "$root/scripts/build.sh"
fi

# The fixture compiles against a sysroot built with the same backend options.
JS_EXTRA_ARGS="js-lane=jir js-opt=on" SKIP_BUILD=1 "$root/scripts/build_sysroot.sh" >/dev/null

work="$(mktemp -d "${TMPDIR:-/tmp}/mires-r3-loops-bounds.XXXXXX")"
trap 'rm -rf "$work"' EXIT

run_js() {
    local engine="$1"
    local program="$2"
    local out="$3"
    {
        printf 'if (typeof console === "undefined") { globalThis.console = { log: (...a) => print(a.join(" ")) }; }\n'
        cat "$root/runtime/shim.js" "$program" "$root/scripts/entry.js"
    } >"$work/harness.js"
    "$engine" "$work/harness.js" >"$out"
}

find_jsc() {
    if command -v jsc >/dev/null 2>&1; then
        command -v jsc
    elif [ -x /System/Library/Frameworks/JavaScriptCore.framework/Helpers/jsc ]; then
        echo /System/Library/Frameworks/JavaScriptCore.framework/Helpers/jsc
    fi
}

echo "=== clean build + witness verification ==="
clean_js="$work/clean.js"
clean_report="$clean_js.lane.json"
RCGJS_WITNESS_CHECK=1 JSC_RUST_MIR_INLINE=on JS_EXTRA_ARGS="js-lane=jir js-opt=on" \
    "$root/scripts/compile_core.sh" "$fixture" "$clean_js"

jq -e '
    (.slice_iter.would_lower // 0) >= 3
    and .slice_iter.final_emitted == .slice_iter.would_lower
    and (.range.asserts_removed // 0) >= 1
    and .witnesses.failed == 0
' "$clean_report" >/dev/null

run_js node "$clean_js" "$work/clean.node.out"
diff -u "$expected" "$work/clean.node.out"

jsc_bin="$(find_jsc || true)"
if [ -n "$jsc_bin" ]; then
    run_js "$jsc_bin" "$clean_js" "$work/clean.jsc.out"
    diff -u "$expected" "$work/clean.jsc.out"
fi

for fault in slice-iterator 1; do
    echo "=== witness fault=$fault rollback ==="
    fault_js="$work/fault_${fault}.js"
    fault_report="$fault_js.lane.json"
    RCGJS_WITNESS_CHECK=1 RCGJS_WITNESS_FAULT="$fault" \
        JSC_RUST_MIR_INLINE=on JS_EXTRA_ARGS="js-lane=jir js-opt=on" \
        "$root/scripts/compile_core.sh" "$fixture" "$fault_js"

    jq -e --arg fault "$fault" '
        .witnesses.failed > 0
        and (if $fault == "slice-iterator" then .slice_iter.final_emitted == 0 else true end)
    ' "$fault_report" >/dev/null

    run_js node "$fault_js" "$work/fault_${fault}.node.out"
    diff -u "$expected" "$work/fault_${fault}.node.out"
done

echo "=== chunks_exact(0) panic negative control ==="
cat >"$work/chunks0.rs" <<EOF
#![no_std]
#![no_main]
#[path = "$root/examples/core-tests/prelude.rs"]
mod prelude;
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let data = [1u8, 2, 3, 4];
    let _ = data.chunks_exact(0);
}
EOF
RCGJS_WITNESS_CHECK=1 JS_EXTRA_ARGS="js-lane=jir js-opt=on" "$root/scripts/compile_core.sh" "$work/chunks0.rs" "$work/chunks0.js"
if run_js node "$work/chunks0.js" "$work/chunks0.out" 2>"$work/chunks0.err"; then
    echo "FAIL: chunks_exact(0) did not trap" >&2
    exit 1
fi

echo "PASS: r3-loops-bounds-test"
