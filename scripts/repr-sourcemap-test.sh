#!/usr/bin/env bash
#
# A representation commit with source maps enabled: the map must describe the replacement that
# was actually emitted, not merely a baseline function from a source-map-only configuration.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT

if [ "${SKIP_BUILD:-0}" != 1 ]; then
	"$script_dir/build.sh" >/dev/null
fi

args='js-lane=jir js-opt=on js-source-map=on js-opt-warnings=off'
JS_EXTRA_ARGS="$args" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >/dev/null
out="$work/89_repr_box_drop.js"
JS_EXTRA_ARGS="$args" "$script_dir/compile_core.sh" \
	"$root/examples/core-tests/89_repr_box_drop.rs" "$out" >/dev/null

node - "$out.lane.json" <<'NODE'
const fs = require("node:fs");
const sidecar = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
if (sidecar.schema !== 17) throw new Error(`sidecar schema is ${sidecar.schema}, expected 17`);
if ((sidecar.repr?.final_emitted?.applied ?? 0) === 0) {
  throw new Error(`source-map build emitted no representation commit: ${JSON.stringify(sidecar.repr)}`);
}
NODE

node "$script_dir/check-sourcemap.js" "$out" 89_repr_box_drop.rs
cat "$root/runtime/shim.js" "$out" "$script_dir/entry.js" | node - \
	| diff -u "$root/examples/core-tests/89_repr_box_drop.expected" -

echo "repr source map: PASS (nonzero final representation commit, valid mappings, correct stdout)"
