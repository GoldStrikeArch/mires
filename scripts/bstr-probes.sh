#!/usr/bin/env bash
#
# The `js-str` differential probes: `examples/bstr-probes/` compiled by stock rustc against `std`
# (the oracle) and by this backend with `js-str=utf16` (the default) and `js-str=bytes`, each run
# under node with `driver.js` as its host. Every line must agree with the oracle under `bytes`;
# a line `utf16` gets wrong is printed, because it is a case the byte strings fix.
#
#   scripts/bstr-probes.sh            # JSC_PANIC selects the mode, as everywhere else
#
# `JS_EXTRA_ARGS` is the base of both runs (default `js-lane=jir js-opt=on`); this script adds
# `js-str=bytes` to it for the second. Set SKIP_BUILD=1 to reuse the built compiler.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
# shellcheck source=sysroot-cache.sh
source "$script_dir/sysroot-cache.sh"
probes="$root/examples/bstr-probes"
work="$root/build/bstr-probes"
rm -rf -- "$work"
mkdir -p "$work"

base=${JS_EXTRA_ARGS:-js-lane=jir js-opt=on}

if [ "${SKIP_BUILD:-0}" != 1 ]; then
	"$script_dir/build.sh" >/dev/null
fi
(cd "$root" && cargo build -p js-extern-macro -p js-macro) >"$work/js-extern-macro.log" 2>&1
case "$(uname -s)" in
Darwin) dylib=dylib ;;
*) dylib=so ;;
esac
proc_macro="$root/target/debug/libjs_extern_macro.$dylib"
js_macro="$root/target/debug/libjs_macro.$dylib"
mkdir -p "$work/proc-macro"
ln -sf "$proc_macro" "$work/proc-macro/$(basename -- "$proc_macro")"

rustc --edition 2021 -O -o "$work/native" "$probes/native.rs"
"$work/native" >"$work/native.out"

status=0
for repr in utf16 bytes; do
	args="$base js-str=$repr"
	JS_EXTRA_ARGS="$args" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >"$work/sysroot-$repr.log" 2>&1
	JS_EXTRA_ARGS="$(jsc_normalize_emit_args "$args")" "$script_dir/compile_core.sh" \
		"$root/mires-js/src/lib.rs" "$work/$repr/libmires_js.rlib" \
		--crate-type rlib --crate-name mires_js \
		--extern js_extern_macro="$proc_macro" >"$work/mires-js-$repr.log" 2>&1
	JS_EXTRA_ARGS="$args" "$script_dir/compile_core.sh" "$probes/probes.rs" "$work/$repr/probes.js" \
		--extern "mires_js=$work/$repr/libmires_js.rlib" --search "$work/proc-macro" \
		>"$work/compile-$repr.log" 2>&1
	cat "$root/runtime/shim.js" "$work/$repr/probes.js" "$probes/driver.js" | node - \
		>"$work/$repr.out" 2>"$work/$repr.err" || {
		echo "js-str=$repr: the program failed:" >&2
		sed 's/^/    /' "$work/$repr.err" >&2
		status=1
		continue
	}
	# The representation probes: what the host is handed, and what Rust measures of what the host
	# hands back, must be the same in both representations (`repr/`).
	JS_EXTRA_ARGS="$args" "$script_dir/compile_core.sh" "$probes/repr/repr.rs" "$work/$repr/repr.js" \
		--extern "mires_js=$work/$repr/libmires_js.rlib" --extern "js_macro=$js_macro" \
		--search "$work/proc-macro" >"$work/compile-repr-$repr.log" 2>&1 &&
		cat "$root/runtime/shim.js" "$work/$repr/repr.js" "$probes/repr/repr.js" | node - \
			>"$work/repr-$repr.out" 2>"$work/repr-$repr.err" || {
		echo "js-str=$repr: the representation probe failed:" >&2
		sed 's/^/    /' "$work/compile-repr-$repr.log" "$work/repr-$repr.err" >&2
		status=1
	}
	total=$(wc -l <"$work/native.out" | tr -d ' ')
	if diff "$work/native.out" "$work/$repr.out" >"$work/$repr.diff"; then
		echo "js-str=$repr: $total/$total lines agree with native Rust"
	else
		differing=$(grep -c '^<' "$work/$repr.diff" || true)
		echo "js-str=$repr: $differing of $total lines differ from native Rust (< native, > js-str=$repr):"
		sed 's/^/    /' "$work/$repr.diff"
		[ "$repr" = bytes ] && status=1
	fi
done
repr_total=$(wc -l <"$work/repr-utf16.out" | tr -d ' ')
if diff "$work/repr-utf16.out" "$work/repr-bytes.out" >"$work/repr.diff"; then
	echo "representation probes: $repr_total/$repr_total host lines identical under utf16 and bytes"
else
	echo "representation probes: utf16 and bytes differ (< utf16, > bytes):"
	sed 's/^/    /' "$work/repr.diff"
	status=1
fi

# A `str` the host could write back through is refused under bytes, with the reason.
if JS_EXTRA_ARGS="$base js-str=bytes" "$script_dir/compile_core.sh" "$probes/repr/refuse.rs" \
	"$work/refuse.js" --extern "mires_js=$work/bytes/libmires_js.rlib" --search "$work/proc-macro" \
	>"$work/refuse.log" 2>&1; then
	echo "refusal probe: compiled under js-str=bytes, but must be refused" >&2
	status=1
elif grep -q 'cannot convert this value at the host boundary' "$work/refuse.log"; then
	echo "refusal probe: refused under js-str=bytes, as expected"
else
	echo "refusal probe: failed for another reason:" >&2
	sed 's/^/    /' "$work/refuse.log" >&2
	status=1
fi
exit "$status"
