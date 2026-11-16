#!/usr/bin/env bash
# Verifies Slice R3-C:
# 1. Boxed-cell promotion/unboxing (`Ord::max`/`min`, mutated `&mut i32` param cells)
# 2. Used-`#[track_caller]` inlining (`chunks_exact`) with exact caller file:line:col on panic
# 3. Trivial 1-op scalar wrapper inlining (`u64::wrapping_mul`) & multi-site `SoleCaller` batching (`i32::wrapping_add`)
# 4. Differential equivalence (`js-opt=off` vs `js-opt=on`) + `RCGJS_WITNESS_CHECK=1`

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT

if [ "${SKIP_BUILD:-0}" != 1 ]; then
	"$script_dir/build.sh" >/dev/null
fi

fix_cells="$root/examples/core-tests/330_r3_inline_cells.rs"
fix_panic="$root/examples/core-tests/331_r3_track_caller_panic.rs"

off_args='js-lane=jir js-opt=off js-opt-warnings=off'
on_args='js-lane=jir js-opt=on js-opt-warnings=off'

JS_EXTRA_ARGS="$off_args" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >/dev/null
JS_EXTRA_ARGS="$off_args" "$script_dir/compile_core.sh" "$fix_cells" "$work/cells_off.js" >/dev/null
JS_EXTRA_ARGS="$off_args" "$script_dir/compile_core.sh" "$fix_panic" "$work/panic_off.js" >/dev/null

JS_EXTRA_ARGS="$on_args" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >/dev/null
RCGJS_WITNESS_CHECK=1 JS_EXTRA_ARGS="$on_args" "$script_dir/compile_core.sh" "$fix_cells" "$work/cells_on.js" >/dev/null
RCGJS_WITNESS_CHECK=1 JS_EXTRA_ARGS="$on_args" "$script_dir/compile_core.sh" "$fix_panic" "$work/panic_on.js" >/dev/null

# 1. Runtime equivalence for 330_r3_inline_cells
cat "$root/runtime/shim.js" "$work/cells_off.js" "$script_dir/entry.js" | node - >"$work/cells_off.out"
cat "$root/runtime/shim.js" "$work/cells_on.js" "$script_dir/entry.js" | node - >"$work/cells_on.out"
diff -u "$work/cells_off.out" "$work/cells_on.out"

node - "$work/cells_on.out" <<'NODE'
const fs = require("node:fs");
const lines = fs.readFileSync(process.argv[2], "utf8").trim().split("\n");
const expected = ["42", "-5", "42", "51", "1009589800"];
if (JSON.stringify(lines) !== JSON.stringify(expected)) {
  throw new Error(`unexpected 330_r3_inline_cells output: ${JSON.stringify(lines)} vs ${JSON.stringify(expected)}`);
}
NODE

# Verify zero JS array boxing (`=[a]`/`=[b]`/`=[]`) and zero un-inlined `chunks_exact`/`wrapping_mul`/`wrapping_add` wrappers in `cells_on.js`.
node - "$work/cells_on.js" "$work/panic_on.js" <<'NODE'
const fs = require("node:fs");
const cellsJs = fs.readFileSync(process.argv[2], "utf8");
if (/=\s*\[\s*\]|=\s*\[\s*[a-zA-Z_$][a-zA-Z0-9_$]*\s*\]/.test(cellsJs)) {
  throw new Error(`found unexpected boxed-cell array allocation in cells_on.js:\n${cellsJs}`);
}
if (/chunks_exact|wrapping_mul|wrapping_add/.test(cellsJs)) {
  throw new Error(`found un-inlined chunks_exact/wrapping_mul/wrapping_add in cells_on.js:\n${cellsJs}`);
}
const panicJs = fs.readFileSync(process.argv[3], "utf8");
if (/chunks_exact/.test(panicJs)) {
  throw new Error(`chunks_exact(0) was not inlined into trigger_chunks_zero in panic_on.js:\n${panicJs}`);
}
NODE

# 2. Verify #[track_caller] exact file:line:col on panic (`331_r3_track_caller_panic.rs` line 27 col 19)
set +e
cat "$root/runtime/shim.js" "$work/panic_off.js" "$script_dir/entry.js" | node - >"$work/panic_off.out" 2>"$work/panic_off.err"
off_rc=$?
cat "$root/runtime/shim.js" "$work/panic_on.js" "$script_dir/entry.js" | node - >"$work/panic_on.out" 2>"$work/panic_on.err"
on_rc=$?
set -e

if [ "$off_rc" -eq 0 ] || [ "$on_rc" -eq 0 ]; then
	echo "r3-inline-cells-test: expected 331_r3_track_caller_panic to abort (off=$off_rc, on=$on_rc)" >&2
	exit 1
fi

diff -u "$work/panic_off.out" "$work/panic_on.out"

node - "$work/panic_on.out" <<'NODE'
const fs = require("node:fs");
const lines = fs.readFileSync(process.argv[2], "utf8").trim().split("\n");
if (lines.length !== 3) {
  throw new Error(`expected 3 panic location lines (file, line, col), got: ${JSON.stringify(lines)}`);
}
const [file, line, col] = lines;
if (!file.endsWith("331_r3_track_caller_panic.rs")) {
  throw new Error(`panic location file was not caller (${file})`);
}
if (line !== "27" || col !== "19") {
  throw new Error(`panic location line:col expected 27:19, got ${line}:${col}`);
}
NODE

echo "r3-inline-cells-test: PASS (unboxed cells, wrapping_mul/add, chunks_exact inlining + exact #[track_caller] panic file:line:col 27:19)"
