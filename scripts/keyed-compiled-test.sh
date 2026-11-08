#!/usr/bin/env bash
#
# Compile one real `keyed_list_v1` view through the JIR lane, then execute it
# against the production DOM/reactive bundle and the transactional keyed shim.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
source "$script_dir/integration-lock.sh"
build_dir="$root/build/keyedtest"
log_dir="$root/build/logs"
dom_module="../../runtime/dom/dist/topcoat-dom.js"
args="js-names=readable js-comments=on js-modules=esm js-shim-module=./shim.mjs js-dom-module=$dom_module js-lane=jir"

mkdir -p "$build_dir" "$log_dir"
node "$script_dir/keyed-transaction.test.mjs"
node "$script_dir/keyed-demo-runtime.mjs" --check

if [ "${SKIP_BUILD:-0}" != 1 ]; then
	"$script_dir/build.sh" >"$log_dir/keyed-build.log" 2>&1
fi

if ! (cd "$root" && cargo build -p view-dom-macro -p js-extern-macro --features keyed-v2) \
	>"$log_dir/keyed-macro.log" 2>&1; then
	cat "$log_dir/keyed-macro.log" >&2
	exit 1
fi
case "$(uname -s)" in
Darwin)
	proc_macro="$root/target/debug/libview_dom_macro.dylib"
	js_extern_macro="$root/target/debug/libjs_extern_macro.dylib"
	;;
*)
	proc_macro="$root/target/debug/libview_dom_macro.so"
	js_extern_macro="$root/target/debug/libjs_extern_macro.so"
	;;
esac

if ! JS_EXTRA_ARGS="$args" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" \
	>"$log_dir/keyed-sysroot.log" 2>&1; then
	cat "$log_dir/keyed-sysroot.log" >&2
	exit 1
fi

abi_rlib="$build_dir/libview_abi.rlib"
if ! JS_EXTRA_ARGS="$args" "$script_dir/compile_core.sh" \
	"$root/view-abi/src/lib.rs" "$abi_rlib" --crate-type rlib \
	>"$log_dir/keyed-abi.log" 2>&1; then
	cat "$log_dir/keyed-abi.log" >&2
	exit 1
fi

node "$script_dir/make-esm-shim.mjs" "$build_dir/shim.mjs"

for name in 01_static 02_live 03_update 04_index 05_bind 06_component 08_selector 09_nested 10_conditional 11_hydration 16_vec 17_bounded_collect; do
	program="$build_dir/$name.mjs"
	if ! JS_EXTRA_ARGS="$args" "$script_dir/compile_core.sh" \
		"$root/examples/keyed-tests/$name.rs" "$program" \
		--extern view_abi="$abi_rlib" --extern view_dom_macro="$proc_macro" \
		>"$log_dir/keyed-$name.log" 2>&1; then
		cat "$log_dir/keyed-$name.log" >&2
		exit 1
	fi
	node "$script_dir/keyed-compiled.test.mjs" "$program"
done

source_rlib="$build_dir/libkeyed_source.rlib"
if ! JS_EXTRA_ARGS="$args" "$script_dir/compile_core.sh" \
	"$root/examples/keyed-tests/support/keyed_source.rs" "$source_rlib" \
	--crate-type rlib --crate-name keyed_source \
	>"$log_dir/keyed-source-lib.log" 2>&1; then
	cat "$log_dir/keyed-source-lib.log" >&2
	exit 1
fi
name=18_cross_crate
program="$build_dir/$name.mjs"
if ! JS_EXTRA_ARGS="$args" "$script_dir/compile_core.sh" \
	"$root/examples/keyed-tests/$name.rs" "$program" \
	--extern view_abi="$abi_rlib" --extern view_dom_macro="$proc_macro" \
	--extern keyed_source="$source_rlib" \
	>"$log_dir/keyed-$name.log" 2>&1; then
	cat "$log_dir/keyed-$name.log" >&2
	exit 1
fi
node "$script_dir/keyed-compiled.test.mjs" "$program"

name=19_host_rows
program="$build_dir/$name.mjs"
host_args="js-names=readable js-comments=on js-modules=esm js-shim-module=../../scripts/keyed-host-shim.mjs js-dom-module=$dom_module js-lane=jir"
if ! JS_EXTRA_ARGS="$host_args" "$script_dir/compile_core.sh" \
	"$root/examples/keyed-tests/$name.rs" "$program" \
	--extern view_abi="$abi_rlib" --extern view_dom_macro="$proc_macro" \
	--extern js_extern_macro="$js_extern_macro" \
	>"$log_dir/keyed-$name.log" 2>&1; then
	cat "$log_dir/keyed-$name.log" >&2
	exit 1
fi
node "$script_dir/keyed-compiled.test.mjs" "$program"

node "$script_dir/keyed-hydration.test.mjs" "$build_dir/11_hydration.mjs"

name=12_hydration_macro
program="$build_dir/$name.mjs"
if ! JS_EXTRA_ARGS="$args" "$script_dir/compile_core.sh" \
	"$root/examples/keyed-tests/$name.rs" "$program" \
	--extern view_abi="$abi_rlib" --extern view_dom_macro="$proc_macro" \
	>"$log_dir/keyed-$name.log" 2>&1; then
	cat "$log_dir/keyed-$name.log" >&2
	exit 1
fi
for mode in matching extra missing wrong-key; do
	node "$script_dir/keyed-hydration-macro.test.mjs" "$program" "$mode"
done

name=13_hydration_range
program="$build_dir/$name.mjs"
if ! JS_EXTRA_ARGS="$args" "$script_dir/compile_core.sh" \
	"$root/examples/keyed-tests/$name.rs" "$program" \
	--extern view_abi="$abi_rlib" --extern view_dom_macro="$proc_macro" \
	>"$log_dir/keyed-$name.log" 2>&1; then
	cat "$log_dir/keyed-$name.log" >&2
	exit 1
fi
for mode in matching extra missing stray-node bad-boundary; do
	node "$script_dir/keyed-hydration-range.test.mjs" "$program" "$mode"
done

name=14_hydration_cardinality
program="$build_dir/$name.mjs"
if ! JS_EXTRA_ARGS="$args" "$script_dir/compile_core.sh" \
	"$root/examples/keyed-tests/$name.rs" "$program" \
	--extern view_abi="$abi_rlib" --extern view_dom_macro="$proc_macro" \
	>"$log_dir/keyed-$name.log" 2>&1; then
	cat "$log_dir/keyed-$name.log" >&2
	exit 1
fi
node "$script_dir/keyed-hydration-cardinality.test.mjs" "$program"

name=15_hydration_nested
program="$build_dir/$name.mjs"
if ! JS_EXTRA_ARGS="$args" "$script_dir/compile_core.sh" \
	"$root/examples/keyed-tests/$name.rs" "$program" \
	--extern view_abi="$abi_rlib" --extern view_dom_macro="$proc_macro" \
	>"$log_dir/keyed-$name.log" 2>&1; then
	cat "$log_dir/keyed-$name.log" >&2
	exit 1
fi
for mode in matching inner-extra; do
	node "$script_dir/keyed-hydration-nested.test.mjs" "$program" "$mode"
done

name=07_macro
program="$build_dir/$name.mjs"
if ! JS_EXTRA_ARGS="$args" "$script_dir/compile_core.sh" \
	"$root/examples/keyed-tests/$name.rs" "$program" \
	--extern view_abi="$abi_rlib" --extern view_dom_macro="$proc_macro" \
	>"$log_dir/keyed-$name.log" 2>&1; then
	cat "$log_dir/keyed-$name.log" >&2
	exit 1
fi
node "$script_dir/keyed-compiled.test.mjs" "$program"

name=20_lifecycle
program="$build_dir/$name.mjs"
if ! JS_EXTRA_ARGS="$args" "$script_dir/compile_core.sh" \
	"$root/examples/keyed-tests/$name.rs" "$program" \
	--extern view_abi="$abi_rlib" --extern view_dom_macro="$proc_macro" \
	>"$log_dir/keyed-$name.log" 2>&1; then
	cat "$log_dir/keyed-$name.log" >&2
	exit 1
fi
node --expose-gc "$script_dir/keyed-lifecycle.test.mjs" "$program"
node --expose-gc "$script_dir/keyed-lifecycle.test.mjs" "$program" --minify

for decline in \
	"90_index_equality_decline:keyed.row-live-index-not-ready:the live index reaches an operation the callable-proxy ABI cannot preserve" \
	"91_item_identity_decline:keyed.row-live-item-use-not-ready:the live item reaches an operation the callable-proxy ABI cannot preserve" \
	"92_iterator_source_decline:keyed.unsupported-source-type:the source is not an array, shared array or slice, or canonical Vec" \
	"93_callable_cleanup_decline:keyed.collection-cleanup-unavailable:callable cleanup is not linked" \
	"94_owned_live_decline:keyed.row-live-item-not-ready:the first live row lane requires an immutable item reference"
do
	IFS=: read -r name reason detail <<<"$decline"
	program="$build_dir/$name.mjs"
	if RCGJS_OPT_DEBUG_FUNC=rust_entry JS_EXTRA_ARGS="$args" "$script_dir/compile_core.sh" \
		"$root/examples/keyed-tests/$name.rs" "$program" \
		--extern view_abi="$abi_rlib" --extern view_dom_macro="$proc_macro" \
		>"$log_dir/keyed-$name.log" 2>&1; then
		echo "expected $name to decline the JIR lane" >&2
		exit 1
	fi
	if ! rg --fixed-strings "$reason: $detail" "$log_dir/keyed-$name.log" >/dev/null; then
		cat "$log_dir/keyed-$name.log" >&2
		exit 1
	fi
	if ! rg --fixed-strings '"fallback_count":1' "$program.lane.json" >/dev/null; then
		cat "$program.lane.json" >&2
		exit 1
	fi
	echo "compiled keyed-list decline: pass ($name.mjs: $reason)"
done
