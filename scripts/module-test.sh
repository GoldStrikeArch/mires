#!/usr/bin/env bash
#
# The module suite: one crate, compiled as a script and as an ES module, has to
# come out the same program.
#
#   scripts/module-test.sh             # every examples/emit fixture
#   scripts/module-test.sh 15_esm      # one
#
# `-Cllvm-args=js-modules=esm` adds two things to a program and changes nothing
# else: the `import` block at the top and the `export { .. }` clause at the
# bottom, both synthesized by the link step. Every item's own JavaScript is byte
# for byte what the script build printed.
#
# That is the property this script exists to hold, and it is worth a suite of its
# own because of what it buys: `js-modules` is then NOT emit-affecting, so a
# crate compiled as a module links against a sysroot built without the option and
# switching modes never costs a sysroot rebuild. If the two ever diverge, the
# `core` in the sysroot and the crate on top of it would print one generic
# instantiation two ways, and link.rs would (rightly) call that two different
# definitions of one item.
#
# For each fixture:
#
#   1. compile it twice, `js-modules=script` and `js-modules=esm`;
#   2. check the module output's shape: the imports come first, the export clause
#      is the last line;
#   3. strip those two and diff what is left against the script output, byte for
#      byte;
#   4. run both -- the script by concatenation, the module through
#      scripts/run-esm.sh -- and diff the two outputs against each other.
#
# Set SKIP_BUILD=1 to reuse an already-built backend dylib.

set -uo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)

emit_dir="$root/examples/emit"
build_dir="$root/build/modules"
log_dir="$root/build/logs"
shim="$root/runtime/shim.js"
entry="$script_dir/entry.js"

# The same baseline the emit goldens use, minus the module option each half sets.
BASE_ARGS="js-names=readable js-comments=on"

mkdir -p "$build_dir" "$log_dir"

if [ "${SKIP_BUILD:-0}" != "1" ]; then
	echo "==> building backend"
	if ! "$script_dir/build.sh" >/dev/null; then
		echo "module-test.sh: backend build failed" >&2
		exit 1
	fi
fi

selected=${1:-}
if [ -n "$selected" ]; then
	name=$(basename -- "$selected")
	name=${name%.rs}
	if [ ! -f "$emit_dir/$name.rs" ]; then
		echo "module-test.sh: no such fixture: $name" >&2
		exit 2
	fi
	sources=$emit_dir/$name.rs
else
	sources=$(ls "$emit_dir"/*.rs 2>/dev/null | sort)
fi

results=""
failures=0
total=0

record() {
	if [ "$2" = FAIL ]; then
		failures=$((failures + 1))
	fi
	results="$results$1 $2 $3"$'\n'
}

for src in $sources; do
	name=$(basename -- "${src%.rs}")

	# A fixture marked NN.core is compiled against the real sysroot rather than
	# against mini_core, which is the one thing this script's compile command
	# cannot do. The invariant it holds -- that an item's own JavaScript is byte
	# for byte identical in the two module modes -- is a property of codegen and
	# not of what a crate depends on, so the mini_core fixtures pin it for every
	# crate in a program, the sysroot's included.
	if [ -f "$emit_dir/$name.core" ]; then
		echo "==> $name (skipped: compiled against the sysroot)"
		continue
	fi

	script_js="$build_dir/$name.script.js"
	module_js="$build_dir/$name.esm.js"
	stripped="$build_dir/$name.stripped.js"
	script_out="$log_dir/module-$name.script.out"
	module_out="$log_dir/module-$name.esm.out"
	log="$log_dir/module-$name.log"
	total=$((total + 1))

	echo "==> $name"

	if ! JS_EXTRA_ARGS="$BASE_ARGS js-modules=script" \
		"$script_dir/compile.sh" "$src" "$script_js" >"$log" 2>&1; then
		echo "    script compile failed:"
		sed 's/^/    /' "$log"
		record "$name" FAIL script-compile-error
		continue
	fi

	if ! JS_EXTRA_ARGS="$BASE_ARGS js-modules=esm" \
		"$script_dir/compile.sh" "$src" "$module_js" >"$log" 2>&1; then
		echo "    module compile failed:"
		sed 's/^/    /' "$log"
		record "$name" FAIL esm-compile-error
		continue
	fi

	# ---------------------------------------------------------------------
	# Shape: imports first, export clause last.
	#
	# The prologue comment is the only thing above the imports, and the
	# `export` line is the last line of the file. Both are checked before the
	# strip below, so that a stray `import` in the middle of the program is a
	# failure rather than something the strip quietly tidies away.
	# ---------------------------------------------------------------------
	first_import=$(grep -n '^import[ *{]' "$module_js" | head -n 1 | cut -d: -f1)
	last_import=$(grep -n '^import[ *{]' "$module_js" | tail -n 1 | cut -d: -f1)
	first_decl=$(grep -n '^\(function\|let\|const\) ' "$module_js" | head -n 1 | cut -d: -f1)
	if [ -n "$first_import" ] && [ -n "$first_decl" ] && [ "$last_import" -gt "$first_decl" ]; then
		echo "    an import comes after the first declaration (line $last_import > $first_decl)"
		record "$name" FAIL import-not-first
		continue
	fi

	exports=$(grep -c '^export[ {]' "$module_js")
	if [ "$exports" -gt 1 ]; then
		echo "    $exports export clauses; there must be at most one"
		record "$name" FAIL many-export-clauses
		continue
	fi
	if [ "$exports" -eq 1 ] && ! tail -n 1 "$module_js" | grep -q '^export[ {]'; then
		echo "    the export clause is not the last line:"
		tail -n 1 "$module_js" | sed 's/^/      /'
		record "$name" FAIL export-not-last
		continue
	fi

	# ---------------------------------------------------------------------
	# Identity: everything else is byte for byte the script build.
	# ---------------------------------------------------------------------
	grep -v '^import[ *{]' "$module_js" | grep -v '^export[ {]' >"$stripped"
	if ! diff -u "$script_js" "$stripped" >"$log_dir/module-$name.diff"; then
		echo "    the items differ between the two modes (- script, + module):"
		sed 's/^/    /' "$log_dir/module-$name.diff"
		record "$name" FAIL items-differ
		continue
	fi

	# ---------------------------------------------------------------------
	# Behaviour: the two programs print the same thing.
	# ---------------------------------------------------------------------
	if ! cat "$shim" "$script_js" "$entry" | node - >"$script_out" 2>"$log"; then
		echo "    the script build failed at run time:"
		sed 's/^/    /' "$log"
		record "$name" FAIL script-runtime-error
		continue
	fi

	if ! "$script_dir/run-esm.sh" "$module_js" "module-$name" >"$module_out" 2>"$log"; then
		echo "    the module build failed at run time:"
		sed 's/^/    /' "$log"
		record "$name" FAIL esm-runtime-error
		continue
	fi

	if ! diff -u "$script_out" "$module_out" >"$log_dir/module-$name.out.diff"; then
		echo "    the two builds printed different things (- script, + module):"
		sed 's/^/    /' "$log_dir/module-$name.out.diff"
		record "$name" FAIL output-differs
		continue
	fi

	imported=${first_import:+yes}
	echo "    ok (imports: ${imported:-none}, exports: $exports)"
	record "$name" PASS "identical, imports ${imported:-none}"
done

echo
echo "================================================================"
printf '%-22s %-8s %s\n' "FIXTURE" "RESULT" "DETAIL"
echo "----------------------------------------------------------------"
printf '%s' "$results" | while read -r name result detail; do
	printf '%-22s %-8s %s\n' "$name" "$result" "$detail"
done
echo "----------------------------------------------------------------"
printf '%d/%d passed\n' "$((total - failures))" "$total"
echo "================================================================"

if [ "$failures" -ne 0 ]; then
	exit 1
fi
