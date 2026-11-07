#!/usr/bin/env bash
#
# Builds the rustc_codegen_mires backend dylib and prints its path.
#
#   scripts/build.sh [extra cargo args...]
#
# The nightly used is pinned by ./rust-toolchain.toml, so plain `cargo` picks it
# up automatically.
#
# Only the backend package is built. The workspace holds crates that have nothing
# to do with compiling Rust to JavaScript (the DOM runtime and its macro), and a
# script whose whole job is to produce the dylib should not fail because one of
# those does not compile.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)

case "$(uname -s)" in
Darwin) dylib="$root/target/release/librustc_codegen_mires.dylib" ;;
*) dylib="$root/target/release/librustc_codegen_mires.so" ;;
esac

cd "$root"
cargo build --release -p rustc_codegen_mires "$@"

if [ ! -f "$dylib" ]; then
	echo "build.sh: cargo succeeded but $dylib is missing" >&2
	echo "build.sh: check that backend/Cargo.toml sets crate-type = [\"dylib\"]" >&2
	exit 1
fi

echo "$dylib"
