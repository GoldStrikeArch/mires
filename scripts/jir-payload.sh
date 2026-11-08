#!/usr/bin/env bash
#
# What the object files spend their bytes on: the format 20 section table, blob dedup, and the
# deflated-against-inflated sizes, for the FingerprintJS boundary rlibs and for any object or rlib
# given on the command line.
#
#   scripts/jir-payload.sh                    # the FingerprintJS boundary rlibs, every built mode
#   scripts/jir-payload.sh path/to/lib.rlib   # these files only
#   scripts/jir-payload.sh --summary ...      # one table row per object instead of key/value lines
#   scripts/jir-payload.sh --build            # build the v2 `off` FingerprintJS artifact first
#
# The port of `research/v2/object_payload_size.py`, which measured the v1 base64 footer. The reader
# is `rcgjs-object/examples/jir-payload.rs` — rustc-free, so it builds in seconds and reads both
# formats: a format 19 member is reported the way the Python did (text, base64, decoded payload),
# a format 20 member by section.
#
# WHERE THE RLIBS ARE
# -------------------
# `experiments/fingerprintjs-rust/tools/build.mjs` writes its build tree under
# `target/js-build/<mode><suffix>/boundary/`, where the suffix is empty for the frozen v1 spike and
# `-rustc-codegen-js-v2` when built with `--compiler-root spike/rustc-codegen-js-v2`. Every
# `off`/`report`/`on` tree of either compiler that exists is reported, and when both compilers'
# `off` trees exist the v2 rlib is held to the Phase 1 gate: no more than twice the v1 `off` rlib.
#
# `--build` runs `node tools/build.mjs --optimizer off --scheduler boundary --compiler-root <v2>`
# first, which builds the backend and its sysroot; the other modes and the v1 tree are built by
# the experiment's own `npm run build*` scripts.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
repo=$(cd -- "$root/../.." && pwd)
experiment="$repo/experiments/fingerprintjs-rust"
build_tree="$experiment/target/js-build"
suffix="-$(basename "$root")"

summary=""
build=""
paths=()
for arg in "$@"; do
	case "$arg" in
	--summary) summary="--summary" ;;
	--build) build=1 ;;
	-h | --help)
		sed -n '2,30p' "$0" | sed 's/^# \{0,1\}//'
		exit 0
		;;
	*) paths+=("$arg") ;;
	esac
done

cd "$root"
cargo build --release -q -p rcgjs-object --example jir-payload
tool="$root/target/release/examples/jir-payload"

if [ -n "$build" ]; then
	(cd "$experiment" && node tools/build.mjs --optimizer off --scheduler boundary --compiler-root "$root")
fi

if [ "${#paths[@]}" -gt 0 ]; then
	exec "$tool" $summary "${paths[@]}"
fi

rlib="boundary/libfingerprintjs_rust_experiment.rlib"
found=0
for mode in off report on; do
	for tree in "$build_tree/$mode" "$build_tree/$mode$suffix"; do
		[ -f "$tree/$rlib" ] || continue
		found=1
		printf '== %s (%s)\n' "$(basename "$tree")" "$tree/$rlib"
		"$tool" $summary "$tree/$rlib"
		if [ -z "$summary" ] && [ -f "$tree/boundary/program.js" ]; then
			printf '%s\tprogram_bytes\t%s\n' "$(basename "$tree")" "$(wc -c <"$tree/boundary/program.js" | tr -d ' ')"
		fi
	done
done
if [ "$found" = 0 ]; then
	echo "jir-payload.sh: no FingerprintJS boundary rlib under $build_tree; run with --build, or pass files" >&2
	exit 1
fi

v1_off="$build_tree/off/$rlib"
v2_off="$build_tree/off$suffix/$rlib"
if [ -f "$v1_off" ] && [ -f "$v2_off" ]; then
	v1_bytes=$(wc -c <"$v1_off" | tr -d ' ')
	v2_bytes=$(wc -c <"$v2_off" | tr -d ' ')
	ratio=$(awk -v a="$v2_bytes" -v b="$v1_bytes" 'BEGIN { printf "%.3f", a / b }')
	printf '\noff rlib: v1 %s bytes, v2 %s bytes, ratio %sx (gate: <= 2.000x)\n' "$v1_bytes" "$v2_bytes" "$ratio"
	if awk -v r="$ratio" 'BEGIN { exit !(r > 2.0) }'; then
		echo "jir-payload.sh: the v2 off rlib exceeds twice the v1 off rlib" >&2
		exit 1
	fi
fi
