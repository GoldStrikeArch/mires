#!/usr/bin/env bash
# Replays the two-statement boundary-corpus shrink whose one caller reaches Vec::deref through a
# native candidate and a baseline value. A caller-wide private-version retarget misreads the
# latter as NativeVec and prints zero; the transactional mixed-site guard keeps both calls sound.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
fixture="$root/examples/core-tests/88_repr_mixed_callsite_version.rs"
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT

if [ "${SKIP_BUILD:-0}" != 1 ]; then
	"$script_dir/build.sh" >/dev/null
fi

# Preserve the minimized corpus case's crate identity: it was generated as `prog.rs`, and exact
# D9 costs include the final item names.
cp "$fixture" "$work/prog.rs"

# Exact costing compresses the final program, including source-location strings. Remap the random
# temporary root so repeated runs price the same bytes.
for mode in off on; do
	JS_EXTRA_ARGS="js-opt=$mode" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >/dev/null
	JSC_REMAP_ROOT="$work" JS_EXTRA_ARGS="js-lane=jir js-opt=$mode js-opt-warnings=off" SKIP_BUILD=1 \
		"$script_dir/compile_core.sh" "$work/prog.rs" "$work/$mode.js" >/dev/null 2>&1
done

node - "$work" "$root" <<'NODE'
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const work = process.argv[2];
const root = process.argv[3];

function run(mode) {
  const output = [];
  const context = vm.createContext({
    TextDecoder,
    TextEncoder,
    console: { log: (value) => output.push(String(value)), error: console.error },
  });
  vm.runInContext(fs.readFileSync(path.join(root, "runtime/shim.js"), "utf8"), context);
  vm.runInContext(fs.readFileSync(path.join(work, `${mode}.js`), "utf8"), context);
  context.rust_entry();
  return output.join("\n");
}

const expected = "3\n12\n2";
const off = run("off");
const on = run("on");
if (off !== expected || on !== expected) {
  throw new Error(`mixed-version differential: off=${JSON.stringify(off)}, on=${JSON.stringify(on)}`);
}

const report = JSON.parse(fs.readFileSync(path.join(work, "on.js.lane.json"), "utf8"));
if (report.repr.planned.would_commit < 1 || report.repr.final_emitted.applied < 1) {
  throw new Error(`fixture did not exercise representation planning: ${JSON.stringify(report.repr)}`);
}
NODE

echo "mixed native/baseline call-site version: PASS"
