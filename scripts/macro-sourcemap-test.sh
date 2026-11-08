#!/usr/bin/env bash
# Check macro invocation, definition and generated-function source locations.
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
source "$script_dir/integration-lock.sh"
work="$root/build/macro-sourcemap"
mkdir -p "$work"
fixture="$root/examples/source_map_macros.rs"
if [ "${SKIP_BUILD:-0}" != 1 ]; then
	"$script_dir/build.sh" >/dev/null
fi
case "$(uname -s)" in
	Darwin) macro="$root/target/debug/libjs_extern_macro.dylib" ;;
	*) macro="$root/target/debug/libjs_extern_macro.so" ;;
esac
(cd "$root" && cargo build -p js-extern-macro >/dev/null)

line_of() {
	awk -v marker="$1" 'index($0, marker) { print NR; exit }' "$fixture"
}

for policy in baseline:off jir:off jir:on; do
	args="js-lane=${policy%:*} js-opt=${policy#*:} js-source-map=on js-opt-warnings=off"
	out="$work/${policy/:/-}.js"
	JS_EXTRA_ARGS="$args" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >/dev/null
	library="$work/libmires_js_${policy/:/-}.rlib"
	JS_EXTRA_ARGS="$args" SKIP_BUILD=1 "$script_dir/compile_core.sh" "$root/mires-js/src/lib.rs" "$library" \
		--crate-type rlib --crate-name mires_js --extern "js_extern_macro=$macro" >/dev/null
	JS_EXTRA_ARGS="$args" SKIP_BUILD=1 "$script_dir/compile_core.sh" "$fixture" "$out" \
		--extern "mires_js=$library" --search "$work" --search "$root/target/debug" >/dev/null
	node "$script_dir/check-sourcemap.js" "$out" source_map_macros.rs 'value === 1' "$(line_of 'collapsed invocation')"
	node "$script_dir/check-sourcemap.js" "$out" source_map_macros.rs 'value === 2' "$(line_of 'if $value == 2')"
	node "$script_dir/check-sourcemap.js" "$out" source_map_macros.rs 'value === 3' "$(line_of 'if value == 3')"
	node "$script_dir/check-sourcemap.js" "$out" source_map_macros.rs 'for (;;)' "$(line_of 'loop {}')"
	panic_call=$(awk '/^    core[$]panicking[$]panic_fmt/ { sub(/^ +/, ""); print; exit }' "$out")
	test -n "$panic_call"
	node "$script_dir/check-sourcemap.js" "$out" source_map_macros.rs "$panic_call" "$(line_of 'panic invocation')"
	{ cat "$root/runtime/shim.js" "$out"; cat <<'JS'
for (const value of [0, 1, 2, 3, 7]) {
  if (collapsed_function(value) !== (value === 1 ? 1 : value + 11)
      || expanded_function(value) !== (value === 2 ? 2 : value + 22)
      || generated_function(value) !== (value === 3 ? 3 : value + 33)) {
    throw new Error('macro fixture result mismatch');
  }
}
if (panic_function(false) !== 99) throw new Error('non-panic result mismatch');
JS
	} | node -
done
echo 'macro source maps: PASS (three compiler policies, exact collapse and body boundaries)'
