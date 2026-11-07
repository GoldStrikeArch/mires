#!/usr/bin/env bash
#
# The size table for `-Cllvm-args=js-minify`.
#
#   scripts/measure-size.sh          # every mode, every fixture
#   scripts/measure-size.sh 01_arith # one fixture
#
# For a fixed set of fixtures — five mini_core tests, three core tests and the
# browser demo — this compiles each one in three modes and prints raw and
# gzipped bytes for all of them:
#
#   readable   the default: full names, indentation, header comments
#   locals     js-minify=locals: compact printing and short *local* names, item
#              names left alone
#   minify     js-minify=on: the above plus short item names, assigned by the
#              link step over the whole program
#
# Gzip, not tokens, is the number that matters: a program is served compressed,
# and a long name repeated a hundred times costs a hundred back-references
# rather than a hundred names. The `locals` column exists so that the two halves
# of the win can be told apart — see the table in CONTRACT.md, which this script
# is the source of.
#
# Set SKIP_BUILD=1 to reuse an already-built backend dylib.
#
# The core fixtures link against build/sysroot, which has to have been built
# with the *same* backend options as the crate linking against it (see the note
# in scripts/test.sh). This script rebuilds it per mode for that reason, so a
# full run costs three sysroot builds; it leaves the sysroot in the default
# mode, which is where scripts/test.sh wants it.

set -uo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)

tests_dir="$root/examples/tests"
core_dir="$root/examples/core-tests"
sysroot="$root/build/sysroot"
out_dir="$root/build/sizes"
log_dir="$root/build/logs"

# name:source-kind pairs. mini_core crates compile standalone; core crates need
# the sysroot; the demo is the one fixture that is neither a test nor `no_std`.
FIXTURES="
01_arith:mini
04_enums:mini
08_closures:mini
12_bigint:mini
14_slices:mini
01_option:core
04_iter_range:core
09_closure:core
counter:demo
"

MODES="readable locals minify"

mkdir -p "$out_dir" "$log_dir"

if [ "${SKIP_BUILD:-0}" != "1" ]; then
	echo "==> building backend"
	"$script_dir/build.sh" >/dev/null || {
		echo "measure-size.sh: backend build failed" >&2
		exit 1
	}
fi

selected=${1:-}

args_for() {
	case $1 in
	readable) printf '' ;;
	locals) printf 'js-minify=locals' ;;
	minify) printf 'js-minify=on' ;;
	esac
}

# The sysroot's `core` is compiled by this backend too, so it carries whatever
# spelling the options asked for. Linking a minified crate against an unminified
# `core` makes the same generic instantiation arrive spelled two ways, which
# link.rs reports as two different definitions of one item.
sync_sysroot() {
	local want=$1 have=""
	[ -d "$sysroot/lib/rustlib/wasm32-unknown-unknown/lib" ] || return 1
	[ -f "$sysroot/.js-args" ] && have=$(cat "$sysroot/.js-args")
	[ "$have" = "$want" ] && return 0
	echo "    (rebuilding the sysroot for JS_EXTRA_ARGS='$want')"
	JS_EXTRA_ARGS="$want" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" \
		>"$log_dir/measure-sysroot.log" 2>&1
}

gzip_size() {
	gzip -9 -c "$1" | wc -c | tr -d ' '
}

rows=""
have_core=1
[ -d "$sysroot/lib/rustlib/wasm32-unknown-unknown/lib" ] || have_core=0

for mode in $MODES; do
	args=$(args_for "$mode")
	echo "==> $mode"

	core_ok=$have_core
	if [ "$have_core" -eq 1 ]; then
		sync_sysroot "$args" || core_ok=0
	fi

	printf '%s\n' "$FIXTURES" | while IFS=: read -r name kind; do
		[ -z "$name" ] && continue
		[ -n "$selected" ] && [ "$name" != "$selected" ] && continue

		case $kind in
		mini) src="$tests_dir/$name.rs" ;;
		core) src="$core_dir/$name.rs" ;;
		demo) src="$root/demo/$name.rs" ;;
		esac
		out="$out_dir/$name.$mode.js"

		if [ "$kind" = core ] && [ "$core_ok" -ne 1 ]; then
			echo "$name $mode - -" >>"$out_dir/.rows"
			continue
		fi

		if [ "$kind" = core ]; then
			compile=$script_dir/compile_core.sh
		else
			compile=$script_dir/compile.sh
		fi

		if JS_EXTRA_ARGS="$args" "$compile" "$src" "$out" \
			>"$log_dir/measure-$name.log" 2>&1; then
			printf '%s %s %s %s\n' "$name" "$mode" \
				"$(wc -c <"$out" | tr -d ' ')" "$(gzip_size "$out")" \
				>>"$out_dir/.rows"
		else
			echo "    $name: compile failed, see $log_dir/measure-$name.log"
			printf '%s %s - -\n' "$name" "$mode" >>"$out_dir/.rows"
		fi
	done
done

# The subshell above cannot export variables, so the rows travel through a file.
rows=$(cat "$out_dir/.rows" 2>/dev/null)
rm -f "$out_dir/.rows"

# Put the sysroot back where scripts/test.sh expects it, so that a plain test run
# after a measurement does not have to rebuild it.
if [ "$have_core" -eq 1 ]; then
	sync_sysroot "" || true
fi

value() {
	printf '%s\n' "$rows" | awk -v n="$1" -v m="$2" -v c="$3" \
		'$1==n && $2==m { print $c; found=1 } END { if (!found) print "-" }'
}

pct() {
	if [ "$1" = "-" ] || [ "$2" = "-" ] || [ "$2" = 0 ]; then
		printf '%s' "-"
	else
		awk -v a="$1" -v b="$2" 'BEGIN { printf "%.0f%%", 100 * (b - a) / b }'
	fi
}

echo
echo "======================================================================================="
printf '%-16s %9s %8s %9s %8s %9s %8s %6s\n' \
	"FIXTURE" "readable" "gzip" "locals" "gzip" "minify" "gzip" "gz cut"
echo "---------------------------------------------------------------------------------------"
printf '%s\n' "$FIXTURES" | while IFS=: read -r name kind; do
	[ -z "$name" ] && continue
	[ -n "$selected" ] && [ "$name" != "$selected" ] && continue
	printf '%-16s %9s %8s %9s %8s %9s %8s %6s\n' \
		"$name" \
		"$(value "$name" readable 3)" "$(value "$name" readable 4)" \
		"$(value "$name" locals 3)" "$(value "$name" locals 4)" \
		"$(value "$name" minify 3)" "$(value "$name" minify 4)" \
		"$(pct "$(value "$name" minify 4)" "$(value "$name" readable 4)")"
done
echo "---------------------------------------------------------------------------------------"
echo "bytes on disk and after gzip -9; 'gz cut' is minify against readable, compressed"
echo "======================================================================================="
