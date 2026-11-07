#!/usr/bin/env bash
#
# Builds the `mires-vanilla` benchmark entry into bench/vanilla/dist/.
#
#   bench/vanilla/build.sh
#
# The bundle is three files concatenated, in this order:
#
#   shim.min.js   the members of runtime/shim.js this program reads, and no others
#   bench.js      src/main.rs compiled by rustc_codegen_mires
#   bootstrap.js  six button listeners and one delegated click
#
# All three are plain scripts, so the page loads one `<script src>` and there is no module
# graph. That is possible because every `#[js_extern]` declaration in src/main.rs is rooted at
# the global scope or at its first argument: nothing is imported, so nothing needs `type=module`.
#
# THE SYSROOT
#
# `js-minify` changes what the backend emits, so the sysroot's `core` has to be built with the
# same setting as the crate linking against it, or one generic instantiation arrives spelled two
# ways and the link step rejects it as two definitions of one item. This flips the sysroot to
# `js-minify=on` and puts it back on the way out, whatever happened in between: the rest of the
# spike's scripts expect it in the default mode. scripts/measure-size.sh does the same dance.

set -euo pipefail

here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$here/../.." && pwd)

dist="$here/dist"
sysroot="$root/build/sysroot"
log_dir="$root/build/logs"

mkdir -p "$dist" "$log_dir"

case "$(uname -s)" in
Darwin) proc_macro="$root/target/debug/libjs_extern_macro.dylib" ;;
*) proc_macro="$root/target/debug/libjs_extern_macro.so" ;;
esac

sync_sysroot() {
	local want=$1 have=""
	[ -f "$sysroot/.js-args" ] && have=$(cat "$sysroot/.js-args")
	[ "$have" = "$want" ] && return 0
	echo "==> rebuilding the sysroot for JS_EXTRA_ARGS='$want'"
	JS_EXTRA_ARGS="$want" SKIP_BUILD=1 "$root/scripts/build_sysroot.sh" \
		>"$log_dir/vanilla-sysroot.log" 2>&1
}

restore_sysroot() {
	sync_sysroot "" || echo "build.sh: WARNING: the sysroot is still in minify mode" >&2
}
trap restore_sysroot EXIT

# ---------------------------------------------------------------------------- the toolchain

echo "==> building the backend"
"$root/scripts/build.sh" >"$log_dir/vanilla-backend.log" 2>&1

# A proc macro runs in the compiler's own process, so `js-extern-macro` is built for the HOST
# whatever `--target` the crate below says.
echo "==> building js-extern-macro for the host"
(cd "$root" && cargo build -p js-extern-macro) >"$log_dir/vanilla-macro.log" 2>&1

sync_sysroot "js-minify=on"

# ---------------------------------------------------------------------------- the program

echo "==> compiling src/main.rs"
JS_EXTRA_ARGS="js-minify=on" "$root/scripts/compile_core.sh" \
	"$here/src/main.rs" "$dist/bench.js" \
	--crate-name mires_vanilla \
	--extern "js_extern_macro=$proc_macro" >/dev/null

if grep -qE '^(import|export)[ {*]' "$dist/bench.js"; then
	echo "build.sh: the program is not a plain script; a declaration named a module" >&2
	exit 1
fi

echo "==> deriving the shim"
node "$here/scripts/make-min-shim.mjs" "$dist/bench.js" "$dist/shim.min.js"

cat "$dist/shim.min.js" "$dist/bench.js" "$here/bootstrap.js" >"$dist/main.js"

# ---------------------------------------------------------------------------- the sizes

echo
printf '%-16s %10s %10s\n' "ARTIFACT" "bytes" "gzip -9"
printf '%s\n' "----------------------------------------"
total_raw=0
total_gz=0
for part in shim.min.js bench.js bootstrap.js; do
	case $part in
	bootstrap.js) file="$here/bootstrap.js" ;;
	*) file="$dist/$part" ;;
	esac
	raw=$(wc -c <"$file" | tr -d ' ')
	gz=$(gzip -9 -c "$file" | wc -c | tr -d ' ')
	total_raw=$((total_raw + raw))
	total_gz=$((total_gz + gz))
	printf '%-16s %10s %10s\n' "$part" "$raw" "$gz"
done
printf '%s\n' "----------------------------------------"
printf '%-16s %10s %10s\n' "sum of parts" "$total_raw" "$total_gz"
printf '%-16s %10s %10s\n' "main.js" \
	"$(wc -c <"$dist/main.js" | tr -d ' ')" \
	"$(gzip -9 -c "$dist/main.js" | wc -c | tr -d ' ')"
echo
echo "==> $dist/main.js"

# Package the krausest delivery: the built program plus the metadata files,
# in the layout bench/run.sh stages into the harness checkout.
pack="$root/bench/vanilla/dist-krausest"
rm -rf "$pack" && mkdir -p "$pack/dist"
cp "$root/bench/vanilla/index.html" "$pack/"
cp "$dist/main.js" "$pack/dist/"
cp "$root/bench/vanilla/krausest/package.json" "$root/bench/vanilla/krausest/package-lock.json" "$pack/"
printf '%s\n' '["index.html", "dist/main.js"]' > "$pack/bench-artifacts.json"
echo "==> packed $pack"
