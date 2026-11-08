#!/usr/bin/env bash

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
compiler_root=$(cd -- "$script_dir/.." && pwd)
repository_root=$(cd -- "$compiler_root/../.." && pwd)
channel=$(sed -n 's/^channel = "\(.*\)"/\1/p' "$compiler_root/rust-toolchain.toml")

if [ -z "$channel" ]; then
	echo "build-procedure-support.sh: compiler toolchain channel is missing" >&2
	exit 1
fi

cargo "+$channel" build --release \
	-p js-macro -p view-dom-macro -p js-extern-macro \
	--manifest-path "$compiler_root/Cargo.toml"
cargo "+$channel" build -p topcoat-runtime-macro --features discover \
	--manifest-path "$repository_root/Cargo.toml" \
	--target-dir "$compiler_root/build/hostmacro-discover"

case "$(uname -s)" in
Darwin) suffix=dylib ;;
*) suffix=so ;;
esac

cp "$compiler_root/build/hostmacro-discover/debug/libtopcoat_runtime_macro.$suffix" \
	"$compiler_root/target/release/libtopcoat_runtime_macro.$suffix"
