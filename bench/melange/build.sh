#!/usr/bin/env bash
#
# Builds the Rust half of the Melange comparison, twice.
#
#   bench/melange/build.sh
#
# Products, all in bench/melange/out/:
#
#   store.readable.js      store.rs compiled with the default names and comments
#   store.minified.js      the same crate with `js-minify=on`
#   run/shim.js            the minimal shim the readable build reaches
#   run/shim.min.js        the minimal shim the minified build reaches
#   run/store.js           a copy of the readable build, beside its shim
#   run/store.min.js       a copy of the minified build, beside its shim
#   run/melange.js         a copy of the extracted Melange output
#
# Both builds use `js-modules=esm`, because node has to be able to `import` the
# result and because that is the only mode a `#[js_extern]` declaration is
# golden-tested in.
#
# THE SYSROOT DANCE
# -----------------
# `js-minify` is emit-affecting: the sysroot's `core` is compiled by this backend
# too, so a minified crate linked against an unminified `core` makes one generic
# instantiation arrive spelled two ways, which link.rs reports as two different
# definitions of one item. So the sysroot is rebuilt per mode, exactly as
# scripts/measure-size.sh does, and a `trap EXIT` puts it back in the default
# mode afterwards even if the build fails partway -- because that is where
# scripts/test.sh expects to find it.
#
# `js-modules` is NOT emit-affecting (CONTRACT.md, "Why `js-modules` is not
# emit-affecting"), so it is passed to the crate alone and never to the sysroot.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/../.." && pwd)

out_dir="$script_dir/out"
run_dir="$out_dir/run"
sysroot="$root/build/sysroot"
log_dir="$root/build/logs"

mkdir -p "$out_dir" "$run_dir" "$log_dir"

case "$(uname -s)" in
Darwin) proc_macro="$root/target/debug/libjs_extern_macro.dylib" ;;
*) proc_macro="$root/target/debug/libjs_extern_macro.so" ;;
esac

# ------------------------------------------------------------- the sysroot mode

# Rebuild only when the mode differs from what is installed, the way
# scripts/measure-size.sh does: a rebuild is minutes and the common case is that
# the sysroot is already right.
sync_sysroot() {
	local want=$1 have=""
	if [ ! -d "$sysroot/lib/rustlib/wasm32-unknown-unknown/lib" ]; then
		echo "build.sh: no sysroot at $sysroot; run scripts/build_sysroot.sh first" >&2
		return 1
	fi
	[ -f "$sysroot/.js-args" ] && have=$(cat "$sysroot/.js-args")
	[ "$have" = "$want" ] && return 0
	echo "==> rebuilding the sysroot for JS_EXTRA_ARGS='$want'"
	JS_EXTRA_ARGS="$want" SKIP_BUILD=1 "$root/scripts/build_sysroot.sh" \
		>"$log_dir/melange-sysroot.log" 2>&1
}

restore_sysroot() {
	local status=$?
	echo "==> restoring the default sysroot"
	sync_sysroot "" || echo "build.sh: could not restore the default sysroot; run scripts/build_sysroot.sh" >&2
	exit "$status"
}

# ------------------------------------------------------------------ the backend

echo "==> building the backend"
"$root/scripts/build.sh" >"$log_dir/melange-backend.log" 2>&1

echo "==> building js-extern-macro for the host"
(cd "$root" && cargo build -p js-extern-macro) >"$log_dir/melange-macro.log" 2>&1

trap restore_sysroot EXIT

# -------------------------------------------------------------------- readable

echo "==> readable"
sync_sysroot ""
JS_EXTRA_ARGS="js-modules=esm" "$root/scripts/compile_core.sh" \
	"$script_dir/store.rs" "$out_dir/store.readable.js" \
	--extern "js_extern_macro=$proc_macro" >/dev/null

# -------------------------------------------------------------------- minified

echo "==> minified"
sync_sysroot "js-minify=on"
JS_EXTRA_ARGS="js-minify=on js-modules=esm" "$root/scripts/compile_core.sh" \
	"$script_dir/store.rs" "$out_dir/store.minified.js" \
	--extern "js_extern_macro=$proc_macro" >/dev/null

# ------------------------------------------------------------------ the run dir

# `type: module` for the whole directory, so a `.js` file with imports is a
# module under node without having to be renamed.
printf '{ "type": "module" }\n' >"$run_dir/package.json"

node "$script_dir/min-shim.mjs" "$out_dir/store.readable.js" "$run_dir/shim.js"
node "$script_dir/min-shim.mjs" "$out_dir/store.minified.js" "$run_dir/shim.min.js"

cp "$out_dir/store.readable.js" "$run_dir/store.js"
cp "$out_dir/store.minified.js" "$run_dir/store.min.js"

if [ ! -f "$out_dir/store.melange.js" ]; then
	echo "build.sh: out/store.melange.js is missing; run \`node extract.mjs\` first" >&2
	exit 1
fi
cp "$out_dir/store.melange.js" "$run_dir/melange.js"

# The minified build's default shim specifier is still `./shim.js`, so it needs
# its own copy under that name in a directory of its own -- otherwise the two
# builds would have to share one shim, which is the opposite of what is being
# measured.
mkdir -p "$run_dir/min"
printf '{ "type": "module" }\n' >"$run_dir/min/package.json"
cp "$run_dir/shim.min.js" "$run_dir/min/shim.js"
cp "$out_dir/store.minified.js" "$run_dir/min/store.js"

echo "==> done"
