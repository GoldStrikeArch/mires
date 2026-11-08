#!/usr/bin/env bash
# Checks that an escaped baseline function and a prepared native caller receive distinct private
# representation bodies, with the versioning propagated through the forwarding callee.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
fixture="$root/examples/core-tests/87_repr_private_version.rs"
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT

if [ "${SKIP_BUILD:-0}" != 1 ]; then
	"$script_dir/build.sh" >/dev/null
fi

# The suite runs on the jir lane (js-lane=jir) by default: W5h deleted the v1 text
# optimizer, so the jir lane is the only lowerer. NATIVE_LANE=baseline checks the
# baseline lane instead, where `js-opt=on` no longer rewrites anything: `on` must
# equal `off` byte for byte.
lane="${NATIVE_LANE:-jir}"

for mode in off report on; do
	args="js-opt=$mode js-opt-passes=native-representations"
	if [ "$lane" = jir ]; then
		args="js-lane=jir $args"
	fi
	JS_EXTRA_ARGS="$args" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >/dev/null
	JS_EXTRA_ARGS="$args js-opt-warnings=off js-opt-report=$work/$mode.json" \
		"$script_dir/compile_core.sh" "$fixture" "$work/$mode.js" >/dev/null
done
cmp "$work/off.js" "$work/report.js"

node - "$work" "$root" "$lane" <<'NODE'
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const work = process.argv[2];
const root = process.argv[3];
const lane = process.argv[4];

function load(mode) {
  const context = vm.createContext({ TextDecoder, TextEncoder });
  vm.runInContext(fs.readFileSync(path.join(root, "runtime/shim.js"), "utf8"), context);
  vm.runInContext(fs.readFileSync(path.join(work, `${mode}.js`), "utf8"), context);
  return context;
}

for (const mode of ["off", "report", "on"]) {
  const context = load(mode);
  const baseline = vm.runInContext(
    "BASELINE_LEN_CALLBACK[0]({ vec: { len: 7 } })",
    context,
  );
  if (baseline !== 7) {
    throw new Error(`${mode}: retained baseline callback returned ${baseline}`);
  }
  if (context.repr_version_result() !== 69) {
    throw new Error(`${mode}: prepared caller returned the wrong result`);
  }
}

if (lane === "jir") {
  const report = JSON.parse(fs.readFileSync(path.join(work, "report.js.lane.json"), "utf8"));
  const on = JSON.parse(fs.readFileSync(path.join(work, "on.js.lane.json"), "utf8"));
  if (report.repr.validated.versions !== 2 || report.repr.final_emitted.applied !== 0) {
    throw new Error(`report mode did not plan and disown both versions: ${JSON.stringify(report.repr)}`);
  }
  if (on.repr.validated.versions !== 2 || on.repr.final_emitted.applied < 1) {
    throw new Error(`on mode did not emit both versions: ${JSON.stringify(on.repr)}`);
  }
  const source = fs.readFileSync(path.join(work, "on.js"), "utf8");
  const callback = source.match(/BASELINE_LEN_CALLBACK = \[([^\]]+)\]/)?.[1];
  if (!callback || !source.includes(`function ${callback}(`) ||
      !source.includes(`function ${callback}$n1(`)) {
    throw new Error("the callback's baseline and private native bodies do not coexist");
  }
}

NODE

if [ "$lane" = baseline ]; then
	# The v1 text optimizer is deleted: `js-opt=on` on the baseline lane no longer rewrites
	# anything, so the receipt is byte-frozen -- `on` must equal `off` exactly.
	cmp "$work/off.js" "$work/on.js"
	echo "native mixed clone (baseline lane): frozen, on == off ($(wc -c <"$work/off.js" | tr -d ' ') bytes)"
else
	off_bytes=$(wc -c <"$work/off.js" | tr -d ' ')
	on_bytes=$(wc -c <"$work/on.js" | tr -d ' ')
	if ((on_bytes >= off_bytes)); then
		echo "native clone lowering (jir lane) did not shrink: $off_bytes -> $on_bytes" >&2
		exit 1
	fi
	echo "native mixed clone (jir lane): PASS ($off_bytes -> $on_bytes bytes)"
fi
