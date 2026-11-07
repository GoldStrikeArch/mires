#!/usr/bin/env bash
#
# Builds a `wasm32-unknown-unknown` sysroot whose `core` was compiled by
# rustc_codegen_mires, so that ordinary `#![no_std]` crates can be compiled to
# JavaScript against the real standard library core.
#
#   scripts/build_sysroot.sh            # incremental (cargo decides)
#   scripts/build_sysroot.sh --clean    # re-copy the sources and rebuild
#
# Set SKIP_BUILD=1 to reuse an already-built backend dylib.
#
# $JS_EXTRA_ARGS is passed on to the backend as `-Cllvm-args`, exactly as
# scripts/compile.sh does. Any option that changes what the backend emits has to
# be given to BOTH this script and the crate build that uses the sysroot, or the
# same instantiation comes out spelled two ways and link.rs rejects the program
# as two different definitions of one item. `js-names` and `js-structure` are the
# ones that bite; see CONTRACT.md. The sysroot is rebuilt from scratch whenever
# these change, because cargo's fingerprint covers rustflags.
#
# What it does
# ------------
#
#   1. copies `library/` out of the pinned toolchain's `rust-src` component into
#      build/stdlib/ — the toolchain's own copy is never touched, so a patch that
#      goes wrong cannot corrupt the installed sources;
#   2. applies every patches/*.patch, in name order, with `patch -p1` rooted at
#      build/stdlib (so a patch's paths start with `library/`);
#   3. builds `library/core`, `library/compiler-builtins` and `library/alloc`
#      with cargo, with `-Zcodegen-backend` pointed at our dylib;
#   4. installs the three rlibs into build/sysroot/, laid out the way `--sysroot`
#      expects.
#
# Why `compiler_builtins` too
# ---------------------------
#
# rustc injects a dependency on it into every `#![no_std]` crate that produces a
# final artifact, so a sysroot with `core` alone is rejected with E0463 before
# codegen ever starts. Nothing in it is ever *called* from emitted JavaScript —
# there are no `memcpy`s in a value model — but the crate has to exist. It is
# cheap: like every rlib it only codegens its non-generic items, and the ones
# this backend cannot express become zombies, which stay silent unless a program
# actually reaches them.
#
# Why `core` and `alloc` and not `library/sysroot`
# ------------------------------------------------
#
# `std` needs an operating system. `core` and `alloc` are what a program in this
# model can have: `alloc` brings `Box`, `Vec` and `String`, and every allocation
# it makes lands on the JavaScript heap the runtime shim keeps (CONTRACT.md,
# "Allocation"). `patches/0002-alloc-default-lib-allocator.patch` is what makes
# that heap the *default* allocator, so a crate using `alloc` needs no
# `#[global_allocator]` of its own.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)

clean=0
for arg in "$@"; do
	case "$arg" in
	--clean) clean=1 ;;
	*)
		echo "usage: build_sysroot.sh [--clean]" >&2
		exit 2
		;;
	esac
done

target=wasm32-unknown-unknown
stdlib="$root/build/stdlib"
sysroot="$root/build/sysroot"
libdir="$sysroot/lib/rustlib/$target/lib"

channel=$(sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\(.*\)".*/\1/p' \
	"$root/rust-toolchain.toml" | head -n 1)
: "${channel:=nightly}"

case "$(uname -s)" in
Darwin) dylib="$root/target/release/librustc_codegen_mires.dylib" ;;
*) dylib="$root/target/release/librustc_codegen_mires.so" ;;
esac

# ---------------------------------------------------------------------------
# 0. the backend itself
# ---------------------------------------------------------------------------

if [ "${SKIP_BUILD:-0}" != "1" ]; then
	echo "==> building backend"
	"$script_dir/build.sh" >/dev/null
fi

if [ ! -f "$dylib" ]; then
	echo "build_sysroot.sh: backend not built: $dylib" >&2
	echo "build_sysroot.sh: run scripts/build.sh first" >&2
	exit 1
fi

# ---------------------------------------------------------------------------
# 1. a private copy of the standard library sources
# ---------------------------------------------------------------------------

src_root=$(rustc "+$channel" --print sysroot)/lib/rustlib/src/rust
if [ ! -d "$src_root/library/core" ]; then
	echo "build_sysroot.sh: no rust-src for $channel at $src_root" >&2
	echo "build_sysroot.sh: run  rustup component add rust-src --toolchain $channel" >&2
	exit 1
fi

if [ "$clean" = 1 ]; then
	rm -rf "$stdlib" "$sysroot"
fi

echo "==> copying $src_root/library"
mkdir -p "$stdlib"
# --delete so a removed patch cannot leave its output behind.
rsync -a --delete \
	--exclude 'target/' \
	"$src_root/library/" "$stdlib/library/"

# ---------------------------------------------------------------------------
# 2. patches
# ---------------------------------------------------------------------------

shopt -s nullglob
patches=("$root"/patches/*.patch)
shopt -u nullglob

if [ ${#patches[@]} -eq 0 ]; then
	echo "==> no patches"
else
	for p in "${patches[@]}"; do
		echo "==> applying $(basename -- "$p")"
		if ! (cd "$stdlib" && patch -p1 --forward --silent <"$p"); then
			echo "build_sysroot.sh: patch failed: $p" >&2
			exit 1
		fi
	done
fi

# ---------------------------------------------------------------------------
# 3. the build
# ---------------------------------------------------------------------------
#
# The flags go in CARGO_TARGET_<TARGET>_RUSTFLAGS rather than RUSTFLAGS on
# purpose: a build script is compiled for the *host*, and handing the host
# compilation a JavaScript codegen backend would produce an executable that
# cannot run. Target-specific rustflags reach only the cross compilation.
#
#   -Zinline-mir             more of core lands as one MIR body, which this
#                            backend turns into one JS function
#   -Cpanic=abort            no unwinding: `Drop` on the panic path is out
#   -Zforce-unstable-if-unmarked  the sysroot crate rule
#   -Ccodegen-units=1        one object per crate, which is what link.rs reads
#   -Cdebuginfo=0            there is no debug info format to emit
#   -Cllvm-args=$JS_EXTRA_ARGS  the backend's own options, when asked for

# One `-Cllvm-args=` per option: rustflags are split on whitespace, so a single
# occurrence carrying two options would arrive as one unknown flag.
backend_args=""
if [ -n "${JS_EXTRA_ARGS:-}" ]; then
	for arg in $JS_EXTRA_ARGS; do
		backend_args="$backend_args -Cllvm-args=$arg"
	done
fi

export CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS="\
-Zcodegen-backend=$dylib \
-Zinline-mir \
-Cpanic=abort \
-Zforce-unstable-if-unmarked \
-Ccodegen-units=1 \
-Coverflow-checks=off \
-Cdebug-assertions=off \
-Cdebuginfo=0$backend_args"

# Almost every dependency is a path dependency inside the copied tree, but not
# quite all: compiler-builtins pulls cfg-if from crates.io. A machine that has
# never built a stdlib has an empty registry cache, so those few are fetched
# while the network is still allowed. With a warm cache this is a no-op.
echo "==> fetching stdlib dependencies"
cargo "+$channel" fetch --manifest-path "$stdlib/library/core/Cargo.toml"
cargo "+$channel" fetch \
	--manifest-path "$stdlib/library/compiler-builtins/compiler-builtins/Cargo.toml"
cargo "+$channel" fetch --manifest-path "$stdlib/library/alloc/Cargo.toml"

# Offline from here on, so the builds below cannot drift from what was just
# fetched by reaching crates.io to refresh the index mid-build.
export CARGO_NET_OFFLINE=true

# Cargo's fingerprint covers the rustc version and the flags, but not the file a
# `-Zcodegen-backend` path points at. Without this the sysroot silently keeps the
# JavaScript an older backend emitted, and every zombie recorded inside `core`
# stays frozen at whatever that build thought was unsupported.
#
# $JS_EXTRA_ARGS is in the stamp too. Cargo does notice a rustflags change, but it
# keeps the rlibs of *both* builds in its deps directory under different metadata
# hashes, and two `libcore-*.rlib` in one sysroot is an error at every later use
# of it. Throwing the build directory away is what keeps exactly one.
stamp="$stdlib/.backend-stamp"
digest=$(printf '%s\n%s' "$(shasum -a 256 "$dylib" | cut -d' ' -f1)" "$backend_args" |
	shasum -a 256 | cut -d' ' -f1)
if [ ! -f "$stamp" ] || [ "$(cat "$stamp")" != "$digest" ]; then
	echo "==> backend or options changed, discarding the previous sysroot build"
	rm -rf "$stdlib/library/target"
fi

start=$(date +%s)

echo "==> building core"
cargo "+$channel" build --release --target "$target" \
	--manifest-path "$stdlib/library/core/Cargo.toml"

echo "==> building compiler_builtins"
cargo "+$channel" build --release --target "$target" \
	--manifest-path "$stdlib/library/compiler-builtins/compiler-builtins/Cargo.toml" \
	--features compiler-builtins,mem

echo "==> building alloc"
cargo "+$channel" build --release --target "$target" \
	--manifest-path "$stdlib/library/alloc/Cargo.toml"

elapsed=$(($(date +%s) - start))
printf '%s' "$digest" >"$stamp"

# ---------------------------------------------------------------------------
# 4. installation
# ---------------------------------------------------------------------------

deps="$stdlib/library/target/$target/release/deps"

rm -rf "$libdir"
mkdir -p "$libdir"

# The newest of each, never every match: a stale rlib from an earlier set of flags
# would make `--sysroot` ambiguous, which rustc rejects outright.
found=0
for pattern in 'libcore-*.rlib' 'libcompiler_builtins-*.rlib' 'liballoc-*.rlib'; do
	newest=$(find "$deps" -maxdepth 1 -name "$pattern" -type f 2>/dev/null |
		tr '\n' '\0' | xargs -0 ls -t 2>/dev/null | head -n 1)
	[ -n "$newest" ] || continue
	cp "$newest" "$libdir/"
	found=$((found + 1))
done

if [ "$found" -lt 3 ]; then
	echo "build_sysroot.sh: expected a core, a compiler_builtins and an alloc rlib in $deps" >&2
	ls -la "$deps" >&2 || true
	exit 1
fi

# The backend options this sysroot was built with, for scripts/test.sh to compare
# against its own $JS_EXTRA_ARGS. An option that changes the JavaScript an item
# prints — js-minify, js-structure, js-queue — has to be the same for every object
# in a program: a generic instantiation codegenned both here and in the crate
# linking against it must come out spelled identically, or link.rs reports the two
# spellings as two different definitions of one item.
printf '%s' "${JS_EXTRA_ARGS:-}" >"$sysroot/.js-args"

echo
echo "==> sysroot: $sysroot"
for rlib in "$libdir"/*.rlib; do
	printf '    %-44s %10d bytes\n' "$(basename -- "$rlib")" "$(wc -c <"$rlib" | tr -d ' ')"
done
printf '    built in %ds\n' "$elapsed"
echo
echo "Compile a no_std crate against it with:"
echo "    scripts/compile_core.sh <src.rs> <out.js>"
