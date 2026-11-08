#!/usr/bin/env bash
set -euo pipefail
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
source "$script_dir/integration-lock.sh"
node "$script_dir/keyed-demo-runtime.mjs" --check
if [ "${SKIP_BUILD:-0}" != 1 ]; then "$script_dir/build.sh" >/dev/null; fi
mkdir -p "$root/build"
work=$(mktemp -d "$root/build/artifact-manifest.XXXXXX")
echo "artifact-manifest: retained output at $work"
export JS_EXTRA_ARGS='js-opt=off js-minify=on js-opt-warnings=off'
SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >"$work/sysroot.log" 2>&1
common="$JS_EXTRA_ARGS js-lane=jir js-modules=esm js-source-map=on js-dts=on"
mkdir -p "$work/chunks" "$work/worker"
JS_EXTRA_ARGS="$common js-chunk=counter:counter js-chunk=search:search js-chunk-role=counter:island js-chunk-role=search:procedure" \
  "$script_dir/compile_core.sh" "$root/examples/artifact_entries.rs" "$work/chunks/inspection.js" >"$work/chunks/compile.log" 2>&1
JS_EXTRA_ARGS="$common js-entry-role=worker js-shim-module=./worker-shim.js" \
  "$script_dir/compile_core.sh" "$root/examples/artifact_entries.rs" "$work/worker/worker.js" --cfg worker >"$work/worker/compile.log" 2>&1
for manifest in "$work/chunks/inspection.js.artifacts.json" "$work/worker/worker.js.artifacts.json"; do
  "$root/target/release/jsc-link" --verify-artifacts "$manifest"
done
node "$script_dir/minimal-shim.mjs" --module --manifest "$work/chunks/inspection.js.artifacts.json" "$root/runtime/shim.js" "$work/chunks/shim.js"
node "$script_dir/minimal-shim.mjs" --module --manifest "$work/worker/worker.js.artifacts.json" "$root/runtime/shim.js" "$work/worker/worker-shim.js"
node "$script_dir/artifact-manifest-test.mjs" "$work"
