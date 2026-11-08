#!/usr/bin/env bash
# Compile the authored counter through its island macro and check entry/signal origins.
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
source "$script_dir/integration-lock.sh"
work="$root/build/island-sourcemap"
mkdir -p "$work"
if [ "${SKIP_BUILD:-0}" != 1 ]; then
	"$script_dir/build.sh" >/dev/null
fi
(cd "$root" && cargo build --locked -p view-dom-macro --target-dir "$work/macros" >/dev/null)
case "$(uname -s)" in
	Darwin) macro="$work/macros/debug/libview_dom_macro.dylib" ;;
	*) macro="$work/macros/debug/libview_dom_macro.so" ;;
esac

# The wrapper selects an existing authored module; it does not expand or rewrite it.
cat > "$work/client.rs" <<RS
#![no_std]
#![no_main]
#![deny(unsafe_code)]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! { loop {} }
#[path = "$root/demo-app/island/counter.rs"]
mod counter;
RS
fixture="$root/demo-app/island/counter.rs"
entry=$(awk '/pub async fn counter\(/ { print NR; exit }' "$fixture")
signal=$(awk '/signal count = start;/ { print NR; exit }' "$fixture")
for policy in baseline:off:off jir:off:off jir:on:off jir:on:on; do
	IFS=: read -r lane opt minify <<< "$policy"
	args="js-lane=$lane js-opt=$opt js-minify=$minify js-modules=esm js-source-map=on js-opt-warnings=off"
	echo "island source maps: $policy"
	out="$work/${policy//:/-}.js"
	JS_EXTRA_ARGS="$args" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >/dev/null
	JS_EXTRA_ARGS="$args" "$script_dir/compile_core.sh" "$root/view-abi/src/lib.rs" "$work/libview_abi.rlib" \
		--crate-type rlib --crate-name view_abi >/dev/null
	JS_EXTRA_ARGS="$args" "$script_dir/compile_core.sh" "$work/client.rs" "$out" \
		--cfg topcoat_client --extern "view_abi=$work/libview_abi.rlib" --extern "view_dom_macro=$macro" >/dev/null
	node "$script_dir/check-sourcemap.js" "$out" island/counter.rs '__island_counter(' "$entry"
	node "$script_dir/check-sourcemap.js" "$out" island/counter.rs '_$createSignal(' "$signal"
done
echo 'island source maps: PASS (four policies, authored entry and signal declaration)'
