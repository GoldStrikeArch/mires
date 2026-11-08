#!/usr/bin/env bash
#
# Event-style host access (examples/host_event_access.rs): listener closures installed through a
# `#[js_extern]` `addEventListener`, reading `event.target.value` and calling `preventDefault`, one
# of them holding a `Drop` guard across a call the check makes throw.
#
#   scripts/host-event-access-test.sh        # SKIP_BUILD=1 reuses the backend
#
# The JIR lane under js-profile=release-balanced (so js-cleanup=on), in optimizer modes off,
# report and on; off and report must emit identical JavaScript and every source map must check.
# scripts/host-event-access-check.mjs runs each program against fake elements and events. Every
# artifact is retained under build/host-event-access.XXXXXX.
set -euo pipefail
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
# shellcheck source=integration-lock.sh
source "$script_dir/integration-lock.sh"
if [ "${SKIP_BUILD:-0}" != 1 ]; then "$script_dir/build.sh" >/dev/null; fi
mkdir -p "$root/build"
work=$(mktemp -d "$root/build/host-event-access.XXXXXX")
echo "host event access: retained output at $work"
(cd "$root" && cargo build -p js-extern-macro --locked) >"$work/macro.log" 2>&1
case "$(uname -s)" in
Darwin) macro="$root/target/debug/libjs_extern_macro.dylib" ;;
*) macro="$root/target/debug/libjs_extern_macro.so" ;;
esac
mkdir -p "$work/proc-macro"
ln -s "$macro" "$work/proc-macro/$(basename -- "$macro")"
node "$script_dir/make-esm-shim.mjs" "$work/shim.mjs"

for mode in off report on; do
	export JS_EXTRA_ARGS="js-profile=release-balanced js-opt=$mode"
	library="$work/library-$mode"
	mkdir -p "$library"
	SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >"$library/sysroot.log" 2>&1
	"$script_dir/compile_core.sh" "$root/mires-js/src/lib.rs" "$library/libmires_js.rlib" \
		--crate-type rlib --crate-name mires_js --extern "js_extern_macro=$macro" >"$library/compile.log" 2>&1
	mkdir -p "$work/jir-$mode"
	program="$work/jir-$mode/program.mjs"
	JS_EXTRA_ARGS="$JS_EXTRA_ARGS js-lane=jir js-modules=esm js-shim-module=../shim.mjs js-source-map=on" \
		"$script_dir/compile_core.sh" "$root/examples/host_event_access.rs" "$program" \
		--extern "mires_js=$library/libmires_js.rlib" --extern "js_extern_macro=$macro" \
		--search "$work/proc-macro" >"$program.compile.log" 2>&1
	if ! node "$script_dir/host-event-access-check.mjs" "$program" >"$program.check.log" 2>&1; then
		cat "$program.check.log"
		echo "jir/$mode: host event access FAILED" >&2
		exit 1
	fi
	echo "jir/$mode: $(tail -n 1 "$program.check.log")"
	node "$script_dir/check-sourcemap.js" "$program" >"$program.map.log"
done
cmp "$work/jir-off/program.mjs" "$work/jir-report/program.mjs"
echo 'host event access: PASS (jir; off, report, on; off == report)'
