#!/usr/bin/env bash
#
# End-to-end `view-abi` + `keyed_list_v1` differential test under `js-str=bytes` and
# `js-profile=release-balanced` in both `unwind` and `abort` panic modes.
#
#   scripts/view-abi-bstr-test.sh            # tests both unwind and abort
#   JSC_PANIC=abort scripts/view-abi-bstr-test.sh
#   SKIP_BUILD=1 scripts/view-abi-bstr-test.sh

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
# shellcheck source=integration-lock.sh
source "$script_dir/integration-lock.sh"

build_root="$root/build/view-abi-bstr"
fixture="$root/examples/view-abi-bstr/fixture.rs"
driver="$root/examples/view-abi-bstr/check.mjs"
panic_modes=${JSC_PANIC:-unwind abort}

mkdir -p "$build_root"

if [ "${SKIP_BUILD:-0}" != 1 ]; then
	echo "==> building backend"
	"$script_dir/build.sh" --locked >"$build_root/build.log" 2>&1
fi

echo "==> building js-extern-macro for the host"
(cd "$root" && cargo build -p js-extern-macro --locked) >"$build_root/macro.log" 2>&1
case "$(uname -s)" in
Darwin) extern_macro="$root/target/debug/libjs_extern_macro.dylib" ;;
*) extern_macro="$root/target/debug/libjs_extern_macro.so" ;;
esac

base_args="js-profile=release-balanced js-modules=esm js-dom-module=./topcoat-dom.js js-shim-module=./shim.mjs"

for panic_mode in $panic_modes; do
	case "$panic_mode" in
	unwind | abort) ;;
	*)
		echo "view-abi-bstr-test.sh: invalid panic mode: $panic_mode" >&2
		exit 2
		;;
	esac

	dir="$build_root/$panic_mode"
	rm -rf "$dir"
	mkdir -p "$dir"
	echo "==> view-abi-bstr [$panic_mode]: sysroot + view-abi"

	if ! JSC_PANIC="$panic_mode" JS_EXTRA_ARGS="js-profile=release-balanced" SKIP_BUILD=1 \
		"$script_dir/build_sysroot.sh" >"$dir/sysroot.log" 2>&1; then
		cat "$dir/sysroot.log" >&2
		exit 1
	fi

	abi_rlib="$dir/libview_abi.rlib"
	if ! JSC_PANIC="$panic_mode" JS_EXTRA_ARGS="$base_args" \
		"$script_dir/compile_core.sh" "$root/view-abi/src/lib.rs" "$abi_rlib" \
		--crate-type rlib --crate-name view_abi >"$dir/view_abi.log" 2>&1; then
		cat "$dir/view_abi.log" >&2
		exit 1
	fi

	printf '{ "type": "module" }\n' >"$dir/package.json"
	node "$script_dir/make-esm-shim.mjs" "$dir/shim.mjs"
	cp "$root/runtime/dom/dist/topcoat-dom.js" "$dir/topcoat-dom.js"

	for variant in utf16 bytes default; do
		case "$variant" in
		utf16)
			extra="$base_args js-str=utf16"
			str_mode=utf16
			;;
		bytes)
			extra="$base_args js-str=bytes"
			str_mode=bytes
			;;
		default)
			extra="$base_args"
			str_mode=bytes
			;;
		esac
		out="$dir/$variant.mjs"
		if ! JSC_PANIC="$panic_mode" JS_EXTRA_ARGS="$extra" \
			"$script_dir/compile_core.sh" "$fixture" "$out" \
			--extern view_abi="$abi_rlib" --extern js_extern_macro="$extern_macro" \
			>"$dir/$variant.compile.log" 2>&1; then
			echo "view-abi-bstr-test.sh: compile failed ($panic_mode/$variant)" >&2
			cat "$dir/$variant.compile.log" >&2
			exit 1
		fi
		fallbacks=$(node -e 'console.log(JSON.parse(require("fs").readFileSync(process.argv[1], "utf8")).fallback_count)' \
			"$out.lane.json")
		if [ "$fallbacks" != 0 ]; then
			echo "view-abi-bstr-test.sh: $panic_mode/$variant had $fallbacks baseline fallback(s)" >&2
			cat "$out.lane.json" >&2
			exit 1
		fi
		node "$driver" --run "$out" "$dir/$variant.json" --str "$str_mode"
	done

	if ! cmp -s "$dir/bytes.mjs" "$dir/default.mjs"; then
		echo "view-abi-bstr-test.sh: $panic_mode default release-balanced output differs from explicit js-str=bytes" >&2
		exit 1
	fi

	node "$driver" --compare "$dir/utf16.json" "$dir/bytes.json" "$dir/default.json"

	# Also verify the minimal tree-shaken ESM shim runs cleanly with the `bytes` build.
	min_dir="$dir/min"
	mkdir -p "$min_dir"
	printf '{ "type": "module" }\n' >"$min_dir/package.json"
	cp "$dir/bytes.mjs" "$min_dir/bytes.mjs"
	cp "$dir/bytes.mjs.artifacts.json" "$min_dir/bytes.mjs.artifacts.json"
	cp "$dir/topcoat-dom.js" "$min_dir/topcoat-dom.js"
	node "$script_dir/minimal-shim.mjs" --module --manifest "$min_dir/bytes.mjs.artifacts.json" \
		"$root/runtime/shim.js" "$min_dir/shim.mjs"
	node "$driver" --run "$min_dir/bytes.mjs" "$min_dir/bytes.json" --str bytes
	node "$driver" --compare --ignore-metrics "$dir/utf16.json" "$min_dir/bytes.json"

	echo "view-abi-bstr [$panic_mode]: PASS"
done
