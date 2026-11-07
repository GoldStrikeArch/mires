#!/usr/bin/env bash
#
# The emit suite: golden files over the JavaScript the backend prints.
#
#   scripts/emit-test.sh                 # check every fixture
#   scripts/emit-test.sh 03_while        # check one
#   UPDATE_EXPECT=1 scripts/emit-test.sh # rewrite the goldens from the output
#
# Each fixture in examples/emit/ is a `#![no_core]` crate against
# examples/mini_core.rs, exactly like examples/tests/, and carries two
# expectations:
#
#   NN.expected      stdout under node, diffed like any other test
#   NN.js.expected   the emitted JavaScript, byte for byte
#
# Both, always. A golden JS file on its own would let a beautifully formatted
# miscompilation through, which is the one failure a printer test must not have.
#
# What is compared is the *filtered* JavaScript:
#
#   * items whose Rust def path starts with one of $EMIT_SKIP are dropped. That
#     is nearly always `mini_core::` — the prelude every fixture drags in is
#     noise, and it changes whenever mini_core does, which would otherwise
#     rebaseline every fixture at once.
#   * the `// <def path>` header comments are dropped with them, so the golden
#     reads as plain JavaScript.
#
# The file that RUNS is the unfiltered one: the filter is a display rule, never
# a compilation one. `js-emit-skip` is spelled here rather than in the backend
# for the same reason — an item the backend left out would be a `ReferenceError`
# at run time, not a tidier golden.
#
# The compile flags are pinned below rather than inherited from $JS_EXTRA_ARGS:
# a golden file is only a golden if it does not depend on the caller's
# environment. Header comments are ON during compilation because they are what
# the filter keys on; they are stripped again before the comparison.
#
# A fixture may carry one more companion file:
#
#   NN.args          extra backend options this fixture is baselined with, on top
#                    of $EMIT_ARGS. Whitespace or newline separated. This is how a
#                    fixture pins the output of an option rather than of a mode
#                    the whole suite runs in.
#   NN.skip          the def path prefixes this fixture hides, replacing
#                    $EMIT_SKIP. Whitespace or newline separated.
#   NN.core          marks a fixture compiled against the real `core` and `alloc`
#                    in build/sysroot (scripts/compile_core.sh) rather than
#                    against mini_core. Such a fixture pins something only the
#                    real standard library produces -- the allocator shim -- and
#                    hides everything else with its own NN.skip.
#   NN.macro         the proc macro crates the fixture uses, one cargo package
#                    name per line. Each is built by plain cargo for the HOST,
#                    because a proc macro runs in the compiler's own process and
#                    rustc loads the host dylib whatever `--target` says, and is
#                    then passed as `--extern`. A fixture using one is nearly
#                    always a NN.core fixture too: an expansion that names
#                    `::core` needs the real one.
#
# A fixture whose args ask for `js-modules=esm` is an ES module, so it runs
# through scripts/run-esm.sh instead of the concatenation below.
#
# A fixture whose args ask for chunks (`js-chunk=<name>:<entry>`) comes out as
# several files, and the chunk names are read straight back out of those args
# rather than repeated in a companion file, so the two cannot disagree. Such a
# fixture carries one more golden per chunk, filtered exactly like the whole
# program:
#
#   NN.<chunk>.js.expected    the chunk the backend wrote beside the output
#   NN.<chunk>.maxbytes       a byte budget for that chunk, unfiltered
#
# The shared chunk (`js-chunk-shared=<name>`, default `shared`) is only written
# when two chunks reach a common item, so it is checked when either the file or
# its golden is there -- which is what makes a split that silently stopped
# happening a failure rather than a pass.
#
#   NN.degenerate    the fixture asks for one chunk holding the whole program, and
#                    that chunk must be the unsplit output byte for byte apart from
#                    the header comment naming the file.
#
# It runs through scripts/run-chunks.sh, which imports each chunk's entry point
# from its own file: that is what proves the cross-chunk imports resolve and that
# the names the chunks import are the names the other chunks export.

set -uo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)

emit_dir="$root/examples/emit"
build_dir="$root/build/emit"
log_dir="$root/build/logs"
shim="$root/runtime/shim.js"
entry="$script_dir/entry.js"

# Def path prefixes whose items are left out of the goldens.
EMIT_SKIP="mini_core"

# The compilation every fixture is baselined under.
EMIT_ARGS="js-names=readable js-comments=on js-emit-skip=$EMIT_SKIP"

mkdir -p "$build_dir" "$log_dir"

if [ "${SKIP_BUILD:-0}" != "1" ]; then
	echo "==> building backend"
	if ! "$script_dir/build.sh" >/dev/null; then
		echo "emit-test.sh: backend build failed" >&2
		exit 1
	fi
fi

selected=${1:-}
if [ -n "$selected" ]; then
	name=$(basename -- "$selected")
	name=${name%.rs}
	if [ ! -f "$emit_dir/$name.rs" ]; then
		echo "emit-test.sh: no such fixture: $name" >&2
		echo "emit-test.sh: available:" >&2
		ls "$emit_dir"/*.rs | while read -r f; do echo "  $(basename -- "${f%.rs}")" >&2; done
		exit 2
	fi
	sources=$emit_dir/$name.rs
else
	sources=$(ls "$emit_dir"/*.rs 2>/dev/null | sort)
	if [ -z "$sources" ]; then
		echo "emit-test.sh: no fixtures found in $emit_dir" >&2
		exit 1
	fi
fi

# filter <file> <skip prefixes>: prints the emitted JS with the skipped items
# and every top level `//` comment removed.
#
# An item starts at column 0 — a header comment, a `function`, a `let` — and
# runs until the next one. Everything a printed body contains is indented, so
# column 0 is an unambiguous item boundary; the only column-0 lines that are not
# item starts are the closing `}` of a function and blank lines, both of which
# belong to the item above.
filter() {
	awk -v skip="$2" '
		BEGIN {
			n = split(skip, prefixes, " ")
		}
		# A header comment names the item that follows it: it decides whether
		# that item is dropped, and `pending` carries the decision across to it.
		/^\/\// {
			path = substr($0, 4)
			drop = 0
			for (i = 1; i <= n; i++) {
				if (index(path, prefixes[i]) == 1) { drop = 1 }
			}
			pending = 1
			next
		}
		# A declaration at column 0. Either the one the comment above introduced,
		# or — with no comment of its own — a link time stub, which is kept.
		/^[A-Za-z_$]/ {
			if (pending) { pending = 0 } else { drop = 0 }
		}
		{ if (!drop) print }
	' "$1" |
		# Collapse the blank lines the dropped items left behind, and drop any
		# leading ones, so the golden starts at the first declaration.
		awk 'NF == 0 { blank = 1; next } { if (blank && seen) print ""; blank = 0; seen = 1; print }'
}

results=""
failures=0
updated=0
total=0

for src in $sources; do
	name=$(basename -- "${src%.rs}")
	out="$build_dir/$name.js"
	expected_out="$emit_dir/$name.expected"
	expected_js="$emit_dir/$name.js.expected"
	actual_out="$log_dir/emit-$name.actual"
	actual_js="$build_dir/$name.filtered.js"
	compile_log="$log_dir/emit-$name.compile.log"
	run_log="$log_dir/emit-$name.run.log"
	total=$((total + 1))

	echo "==> $name"

	skip=$EMIT_SKIP
	if [ -f "$emit_dir/$name.skip" ]; then
		skip=$(tr '\n' ' ' <"$emit_dir/$name.skip")
	fi

	args="js-names=readable js-comments=on js-emit-skip=$skip"
	if [ -f "$emit_dir/$name.args" ]; then
		args="$args $(tr '\n' ' ' <"$emit_dir/$name.args")"
	fi

	# A fixture marked NN.core is compiled against the real sysroot.
	compile=compile.sh
	if [ -f "$emit_dir/$name.core" ]; then
		compile=compile_core.sh
	fi

	# The proc macros the fixture uses, built for the host and handed over as externs.
	externs=()
	macro_failed=0
	if [ -f "$emit_dir/$name.macro" ]; then
		for package in $(tr '\n' ' ' <"$emit_dir/$name.macro"); do
			crate=${package//-/_}
			case "$(uname -s)" in
			Darwin) dylib="$root/target/debug/lib$crate.dylib" ;;
			*) dylib="$root/target/debug/lib$crate.so" ;;
			esac
			if ! (cd "$root" && cargo build -p "$package") \
				>"$log_dir/emit-$name.macro.log" 2>&1; then
				echo "    could not build the proc macro $package:"
				sed 's/^/    /' "$log_dir/emit-$name.macro.log"
				macro_failed=1
				break
			fi
			externs+=(--extern "$crate=$dylib")
		done
	fi
	if [ "$macro_failed" -ne 0 ]; then
		results="$results$name FAIL macro-build"$'\n'
		failures=$((failures + 1))
		continue
	fi

	# The chunks this fixture asks for, read back out of the args it is
	# baselined with: `js-chunk=<name>:<entry>` pairs, and the shared chunk's
	# name. A chunk file left over from an earlier compilation would resolve at
	# run time and would be the wrong file, so they go first.
	chunk_pairs=$(printf '%s\n' $args | sed -n 's/^js-chunk=//p')
	shared_name=$(printf '%s\n' $args | sed -n 's/^js-chunk-shared=//p' | tail -n 1)
	: "${shared_name:=shared}"
	chunk_names=""
	for pair in $chunk_pairs; do
		chunk_names="$chunk_names ${pair%%:*}"
	done
	if [ -n "$chunk_pairs" ]; then
		for chunk in $chunk_names "$shared_name"; do
			rm -f "$build_dir/$chunk.js" "$build_dir/$chunk.js.map"
		done
	fi

	if ! JS_EXTRA_ARGS="$args" SKIP_BUILD=1 "$script_dir/$compile" "$src" "$out" \
		${externs[@]+"${externs[@]}"} >"$compile_log" 2>&1; then
		echo "    compile failed:"
		sed 's/^/    /' "$compile_log"
		results="$results$name FAIL compile-error"$'\n'
		failures=$((failures + 1))
		continue
	fi

	# The runtime half. The file that runs is the unfiltered one.
	if [ -n "$chunk_pairs" ]; then
		run_cmd=("$script_dir/run-chunks.sh" "$out" "emit-$name" $chunk_pairs)
	else
		case " $args " in
		*js-modules=esm*) run_cmd=("$script_dir/run-esm.sh" "$out" "emit-$name") ;;
		*) run_cmd=() ;;
		esac
	fi
	if [ "${#run_cmd[@]}" -eq 0 ]; then
		cat "$shim" "$out" "$entry" | node - >"$actual_out" 2>"$run_log"
		status=$?
	else
		"${run_cmd[@]}" >"$actual_out" 2>"$run_log"
		status=$?
	fi
	if [ "$status" -ne 0 ]; then
		echo "    runtime error:"
		sed 's/^/    /' "$run_log"
		results="$results$name FAIL runtime-error"$'\n'
		failures=$((failures + 1))
		continue
	fi

	filter "$out" "$skip" >"$actual_js"

	# What this fixture compares: the whole program, and one more pair per chunk.
	# The shared chunk is in the list when either the file or its golden is
	# there, so a split that stopped producing one is a failure and not a pass.
	labels=("js")
	actuals=("$actual_js")
	expectations=("$expected_js")
	missing_chunk=""
	for chunk in $chunk_names; do
		if [ ! -f "$build_dir/$chunk.js" ]; then
			missing_chunk=$chunk
			break
		fi
	done
	if [ -z "$missing_chunk" ] && [ -n "$chunk_pairs" ]; then
		for chunk in $chunk_names "$shared_name"; do
			chunk_file="$build_dir/$chunk.js"
			chunk_expected="$emit_dir/$name.$chunk.js.expected"
			if [ ! -f "$chunk_file" ] && [ ! -f "$chunk_expected" ]; then
				continue
			fi
			if [ ! -f "$chunk_file" ]; then
				missing_chunk=$chunk
				break
			fi
			chunk_actual="$build_dir/$name.$chunk.filtered.js"
			filter "$chunk_file" "$skip" >"$chunk_actual"
			labels+=("$chunk")
			actuals+=("$chunk_actual")
			expectations+=("$chunk_expected")
		done
	fi
	if [ -n "$missing_chunk" ]; then
		echo "    the backend wrote no chunk '$missing_chunk' beside $out"
		results="$results$name FAIL missing-chunk"$'\n'
		failures=$((failures + 1))
		continue
	fi

	if [ "${UPDATE_EXPECT:-0}" = "1" ]; then
		changed=""
		if ! cmp -s "$actual_out" "$expected_out" 2>/dev/null; then
			cp "$actual_out" "$expected_out"
			changed="stdout"
		fi
		for index in "${!actuals[@]}"; do
			if ! cmp -s "${actuals[$index]}" "${expectations[$index]}" 2>/dev/null; then
				cp "${actuals[$index]}" "${expectations[$index]}"
				changed="${changed:+$changed+}${labels[$index]}"
			fi
		done
		if [ -n "$changed" ]; then
			echo "    updated ($changed)"
			results="$results$name UPDATED $changed"$'\n'
			updated=$((updated + 1))
		else
			echo "    ok (unchanged)"
			results="$results$name PASS unchanged"$'\n'
		fi
		continue
	fi

	absent=""
	if [ ! -f "$expected_out" ]; then
		absent="stdout"
	fi
	for index in "${!expectations[@]}"; do
		if [ ! -f "${expectations[$index]}" ]; then
			absent="${absent:+$absent+}${labels[$index]}"
		fi
	done
	if [ -n "$absent" ]; then
		echo "    missing expectation ($absent); run UPDATE_EXPECT=1 scripts/emit-test.sh $name"
		results="$results$name FAIL no-expectation"$'\n'
		failures=$((failures + 1))
		continue
	fi

	if ! diff -u "$expected_out" "$actual_out" >"$log_dir/emit-$name.diff"; then
		echo "    stdout mismatch (- expected, + actual):"
		sed 's/^/    /' "$log_dir/emit-$name.diff"
		results="$results$name FAIL output-mismatch"$'\n'
		failures=$((failures + 1))
		continue
	fi

	mismatch=""
	for index in "${!actuals[@]}"; do
		label=${labels[$index]}
		diff_log="$log_dir/emit-$name.$label.diff"
		if [ "$label" = "js" ]; then
			diff_log="$log_dir/emit-$name.js.diff"
		fi
		if ! diff -u "${expectations[$index]}" "${actuals[$index]}" >"$diff_log"; then
			echo "    emitted JavaScript changed ($label) (- expected, + actual):"
			sed 's/^/    /' "$diff_log"
			mismatch=$label
			break
		fi
	done
	if [ -n "$mismatch" ]; then
		echo "    if this is an improvement: UPDATE_EXPECT=1 scripts/emit-test.sh $name"
		results="$results$name FAIL js-mismatch"$'\n'
		failures=$((failures + 1))
		continue
	fi

	# A fixture that asks for one chunk over the program's only root asks for the
	# program back: the header comment names the chunk instead of the program, and
	# nothing else may move.
	if [ -f "$emit_dir/$name.degenerate" ]; then
		only=${chunk_names# }
		if ! diff -u <(tail -n +2 "$out") <(tail -n +2 "$build_dir/$only.js") \
			>"$log_dir/emit-$name.degenerate.diff"; then
			echo "    the one chunk is not the whole program (- program, + chunk):"
			sed 's/^/    /' "$log_dir/emit-$name.degenerate.diff"
			results="$results$name FAIL not-degenerate"$'\n'
			failures=$((failures + 1))
			continue
		fi
	fi

	# The budget is on the file that runs, not on the filtered display form: a
	# chunk's cost is what a page has to download.
	over=""
	for chunk in $chunk_names "$shared_name"; do
		budget_file="$emit_dir/$name.$chunk.maxbytes"
		if [ ! -f "$budget_file" ] || [ ! -f "$build_dir/$chunk.js" ]; then
			continue
		fi
		budget=$(tr -dc '0-9' <"$budget_file")
		chunk_size=$(wc -c <"$build_dir/$chunk.js" | tr -d ' ')
		if [ -n "$budget" ] && [ "$chunk_size" -gt "$budget" ]; then
			echo "    chunk '$chunk' over budget: $chunk_size bytes > $budget (see $budget_file)"
			over=$chunk
		fi
	done
	if [ -n "$over" ]; then
		results="$results$name FAIL over-budget"$'\n'
		failures=$((failures + 1))
		continue
	fi

	size=$(wc -c <"$actual_js" | tr -d ' ')
	detail="$size bytes"
	if [ -n "$chunk_pairs" ]; then
		for chunk in $chunk_names "$shared_name"; do
			if [ -f "$build_dir/$chunk.js" ]; then
				chunk_size=$(wc -c <"$build_dir/$chunk.js" | tr -d ' ')
				detail="$detail $chunk=$chunk_size"
			fi
		done
	fi
	echo "    ok ($detail of JavaScript)"
	results="$results$name PASS $detail"$'\n'
done

echo
echo "================================================================"
printf '%-22s %-8s %s\n' "FIXTURE" "RESULT" "DETAIL"
echo "----------------------------------------------------------------"
printf '%s' "$results" | while read -r name result detail; do
	printf '%-22s %-8s %s\n' "$name" "$result" "$detail"
done
echo "----------------------------------------------------------------"
if [ "$updated" -ne 0 ]; then
	printf '%d/%d passed, %d rebaselined\n' "$((total - failures - updated))" "$total" "$updated"
else
	printf '%d/%d passed\n' "$((total - failures))" "$total"
fi
echo "================================================================"

if [ "$failures" -ne 0 ]; then
	exit 1
fi
