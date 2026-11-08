#!/usr/bin/env bash
#
# The shadow lane: compile every fixture twice from one set of objects and prove the two
# programs behave identically.
#
#   scripts/lane-shadow-test.sh                    # every suite
#   scripts/lane-shadow-test.sh tests core-tests   # some of them
#
# `-Cllvm-args=js-lane=shadow` makes the link step print the program twice: the baseline (v1's
# text, item for item) to the output path, and the JIR lane's — every item the lowerer took,
# every item it declined still on v1's text — to `<out>.jir.js`, with `<out>.lane.json` saying
# which items went which way. This script runs BOTH files the way the fixture's own suite runs
# one, and diffs what came out.
#
# That is the whole safety argument for a per item lane, and it is why this script exists rather
# than a golden: byte equality is not the property (a lane may print better JavaScript), stdout
# equality is. A fixture is
#
#   PASS            at least one item took the JIR lane and the two programs agree
#   FALLBACK-ONLY   no item took the JIR lane, and the two programs are byte identical
#   DIFF            the two disagree — on stdout, on the trace, on the exit code, or on their
#                   bytes when no item was lowered at all
# and any DIFF fails the run. With the lowerer stubbed out every fixture is FALLBACK-ONLY, which
# is the point of the stub: total fallback is a working compiler, and the number that has to move
# is the fallback count and nothing else.
#
# THE SUITES, AND HOW EACH ONE RUNS A PROGRAM
# -------------------------------------------
#   tests        examples/tests, scripts/compile.sh, run as `shim + program + entry` under node
#   core-tests   examples/core-tests against the real `core`, run the same way. `.expect_abort`
#                fixtures exit non-zero on purpose, which is why the exit code is compared rather
#                than required to be zero
#   dom-tests    examples/dom-tests as ES modules, run through scripts/dom-trace.mjs against the
#                contract's recording stub; the trace is the output that is diffed
#   async-tests  the executor fixtures, whose runner is scripts/async-check.mjs over the whole
#                build directory rather than a command per program. So this suite is compared by
#                running scripts/async-test.sh twice — once plain, once with `js-lane=jir` — and
#                diffing the behaviour log the driver wrote. Same question, one verdict for the
#                suite instead of one per fixture
#
# `js-lane` is not emit-affecting (`rcgjs-object/src/opts.rs`), so every suite links against the
# ordinary default sysroot and none of this costs a rebuild.
#
# Every compile and every run is under a watchdog ($JSC_LANE_TIMEOUT seconds, default 300): a
# lowerer that does not terminate has to name the fixture it hung on rather than hang the run.
# Such a fixture is reported ERROR compile-timeout and fails the suite.
#
# The run ends with the fallback histogram: how many items each decline code and each `ItemClass`
# sent back to the baseline, over every sidecar written. That is the worklist a promotion round
# reads.

set -uo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
# shellcheck source=sysroot-cache.sh
source "$script_dir/sysroot-cache.sh"

build_dir="$root/build/laneshadow"
log_dir="$root/build/logs"
shim="$root/runtime/shim.js"
entry="$script_dir/entry.js"

ALL_SUITES="tests core-tests dom-tests async-tests"
suites=${*:-$ALL_SUITES}
for suite in $suites; do
	case " $ALL_SUITES " in
	*" $suite "*) ;;
	*)
		echo "lane-shadow-test.sh: unknown suite: $suite" >&2
		echo "lane-shadow-test.sh: available: $ALL_SUITES" >&2
		exit 2
		;;
	esac
done

case "$(uname -s)" in
Darwin)
	dylib="$root/target/release/librustc_codegen_mires.dylib"
	macro_suffix=dylib
	;;
*)
	dylib="$root/target/release/librustc_codegen_mires.so"
	macro_suffix=so
	;;
esac

rm -rf "$build_dir"
mkdir -p "$build_dir" "$log_dir"

if [ "${SKIP_BUILD:-0}" != "1" ]; then
	echo "==> building backend"
	if ! "$script_dir/build.sh" >/dev/null; then
		echo "lane-shadow-test.sh: backend build failed" >&2
		exit 1
	fi
fi

results=""
failures=0
total=0

# record <name> <verdict> <detail>
record() {
	results="$results$1 $2 $3"$'\n'
	total=$((total + 1))
	if [ "$2" = DIFF ] || [ "$2" = ERROR ]; then
		failures=$((failures + 1))
	fi
}

# lane_count <sidecar> <field>: `jir` or `fallback` rows in a lane sidecar, 0 when there is none.
lane_count() {
	[ -f "$1" ] || {
		printf '0'
		return
	}
	node -e '
		const fs = require("node:fs")
		const doc = JSON.parse(fs.readFileSync(process.argv[1], "utf8"))
		const want = process.argv[2]
		process.stdout.write(String((doc.items ?? []).filter((i) => i.lane === want).length))
	' "$1" "$2"
}

# verdict <name> <program> <jir program> <sidecar> <baseline out> <jir out> <status> <jir status>
#
# The one comparison every suite makes, whatever "out" happens to be for it: stdout for a script
# mode program, the recorded trace for a DOM fixture.
verdict() {
	local name=$1 program=$2 jir=$3 sidecar=$4
	local baseline_out=$5 jir_out=$6 baseline_status=$7 jir_status=$8
	local lowered slug
	# A suite-qualified name holds a `/`, which is not a file name.
	slug=${name//\//-}
	lowered=$(lane_count "$sidecar" jir)

	# Fixture 78's phase-4 row 7 DIFF (a) was the v1 text optimizer's miscompile; W5h deleted the
	# optimizer and the disagreement with it, so the fixture is an ordinary row again and every
	# disagreement fails the run.
	if [ "$baseline_status" != "$jir_status" ]; then
		echo "    exit code differs: baseline $baseline_status, jir $jir_status"
		record "$name" DIFF "exit-$baseline_status-vs-$jir_status"
		return
	fi
	if ! diff -u "$baseline_out" "$jir_out" >"$log_dir/laneshadow-$slug.diff"; then
		echo "    output differs (- baseline, + jir):"
		sed 's/^/    /' "$log_dir/laneshadow-$slug.diff" | head -n 40
		record "$name" DIFF output
		return
	fi
	if [ "$lowered" -eq 0 ]; then
		# Nothing was lowered, so the two files have to be the same file. This is what catches a
		# driver that rewrote an item it never counted.
		if ! cmp -s "$program" "$jir"; then
			echo "    no item took the JIR lane, but the two programs differ"
			record "$name" DIFF bytes-differ
			return
		fi
		record "$name" FALLBACK-ONLY "$(lane_count "$sidecar" fallback) fallbacks"
		echo "    ok (no item lowered, programs identical)"
		return
	fi
	record "$name" PASS "$lowered lowered, $(lane_count "$sidecar" fallback) fallbacks"
	echo "    ok ($lowered items lowered)"
}

# with_timeout <seconds> <command...>: runs a command and kills it if it overruns.
#
# A lowerer is a program that walks a graph, and the two ways one goes wrong are a wrong answer
# and no answer. This suite is the differential for the first; the watchdog is so the second
# reports the fixture that hung instead of hanging the run. macOS ships no coreutils `timeout`,
# so it is a background child and a killer beside it. Exit status 124 is the overrun, spelled the
# way `timeout(1)` spells it.
JSC_LANE_TIMEOUT=${JSC_LANE_TIMEOUT:-300}

with_timeout() {
	local seconds=$1
	shift
	"$@" &
	local pid=$!
	(
		sleep "$seconds"
		kill -9 "$pid" 2>/dev/null
	) &
	local watchdog=$!
	wait "$pid"
	local status=$?
	kill "$watchdog" 2>/dev/null
	wait "$watchdog" 2>/dev/null
	if [ "$status" -ge 128 ]; then
		return 124
	fi
	return "$status"
}

# run_script_mode <program> <stdout file> : the `shim + program + entry` concatenation.
run_script_mode() {
	cat "$shim" "$1" "$entry" | node - >"$2" 2>"$3"
}

# The optimizer mode both lanes run under. `SHADOW_OPT_MODE=on` is the differential that says the
# JIR passes (jsc-link/src/opt/) and the frozen v1 text optimizer still agree item for item: the
# baseline half of a shadow link gets v1's rewrites, the JIR half gets the passes, and this script
# runs both programs. `js-opt` IS emit-affecting, so the mode asks for its own sysroot.
SHADOW_OPT_MODE=${SHADOW_OPT_MODE:-off}
case "$SHADOW_OPT_MODE" in
off) OPT_ARGS="" ;;
report | on) OPT_ARGS="js-opt=$SHADOW_OPT_MODE" ;;
*)
	echo "lane-shadow-test.sh: SHADOW_OPT_MODE must be off, report, or on" >&2
	exit 2
	;;
esac

# ----------------------------------------------------------------------- the sysroot

# ensure_sysroot <emit-affecting options>: the keyed sysroot those options need, built if absent.
#
# The sysroot is keyed on the backend dylib and on the emit-affecting options only, exactly as
# scripts/compile_core.sh resolves it. `js-lane` is not one of them — an object carries both lanes
# — so a shadow run links against the sysroot an ordinary run already built. What DOES key one is
# a suite spelling out an emit-affecting default, which is why the async suite (whose script
# passes `js-names=readable`) asks for its own and the DOM suite here does not spell it.
sysroot_checked=""

ensure_sysroot() {
	local args path
	args=$(jsc_normalize_emit_args "${1:-}")
	case " $sysroot_checked " in
	*"|$args|"*) return 0 ;;
	esac
	path=$(JSC_SYSROOT_ARGS="$args" jsc_sysroot_path "$root" wasm32-unknown-unknown "$dylib") || return 1
	if [ ! -d "$path/lib/rustlib/wasm32-unknown-unknown/lib" ]; then
		echo "==> building the sysroot for '$args' (about a minute)"
		if ! SKIP_BUILD=1 JS_EXTRA_ARGS="$args" "$script_dir/build_sysroot.sh" \
			>"$log_dir/laneshadow-sysroot.log" 2>&1; then
			echo "    failed, see $log_dir/laneshadow-sysroot.log"
			return 1
		fi
	fi
	sysroot_checked="$sysroot_checked |$args|"
	return 0
}

# ------------------------------------------------------------------------ examples/tests

run_tests_suite() {
	echo "==> examples/tests"
	local out dir
	dir="$build_dir/tests"
	mkdir -p "$dir"
	local src name compile_status
	for src in "$root"/examples/tests/*.rs; do
		[ -f "$src" ] || continue
		# An expected-failure fixture is a diagnostic, not a program.
		[ -f "${src%.rs}.expect_fail" ] && continue
		name=$(basename -- "${src%.rs}")
		out="$dir/$name.js"
		echo "==> $name"
		JS_EXTRA_ARGS="${OPT_ARGS:+$OPT_ARGS }js-lane=shadow" SKIP_BUILD=1 with_timeout "$JSC_LANE_TIMEOUT" \
			"$script_dir/compile.sh" "$src" "$out" \
			>"$log_dir/laneshadow-$name.compile.log" 2>&1
		compile_status=$?
		if [ "$compile_status" -eq 124 ]; then
			echo "    compile did not finish within ${JSC_LANE_TIMEOUT}s"
			record "$name" ERROR compile-timeout
			continue
		elif [ "$compile_status" -ne 0 ]; then
			echo "    compile failed, see $log_dir/laneshadow-$name.compile.log"
			record "$name" ERROR compile
			continue
		fi
		with_timeout "$JSC_LANE_TIMEOUT" run_script_mode "$out" \
			"$dir/$name.baseline.out" "$dir/$name.baseline.err"
		local baseline_status=$?
		with_timeout "$JSC_LANE_TIMEOUT" run_script_mode "$out.jir.js" \
			"$dir/$name.jir.out" "$dir/$name.jir.err"
		local jir_status=$?
		verdict "$name" "$out" "$out.jir.js" "$out.lane.json" \
			"$dir/$name.baseline.out" "$dir/$name.jir.out" "$baseline_status" "$jir_status"
	done
}

# ------------------------------------------------------------------- examples/core-tests

run_core_tests_suite() {
	echo "==> examples/core-tests"
	if ! ensure_sysroot "$OPT_ARGS"; then
		echo "    skipped: no sysroot"
		record core-tests SKIP no-sysroot
		return
	fi
	local dir="$build_dir/core-tests"
	local macro_dir="$dir/proc-macro"
	mkdir -p "$dir" "$macro_dir"

	# `mires-js` and its proc macro, exactly as scripts/test.sh offers them to every core test.
	if ! (cd "$root" && cargo build -p js-extern-macro) >"$log_dir/laneshadow-core-macro.log" 2>&1; then
		echo "    skipped: js-extern-macro did not build"
		record core-tests SKIP no-macro
		return
	fi
	local proc_macro="$root/target/debug/libjs_extern_macro.$macro_suffix"
	cp "$proc_macro" "$macro_dir/"
	local rlib="$dir/libmires_js.rlib"
	if ! JS_EXTRA_ARGS="$OPT_ARGS" SKIP_BUILD=1 "$script_dir/compile_core.sh" \
		"$root/mires-js/src/lib.rs" "$rlib" --crate-type rlib --crate-name mires_js \
		--extern "js_extern_macro=$proc_macro" >"$log_dir/laneshadow-core-mires-js.log" 2>&1; then
		echo "    skipped: mires-js did not build"
		record core-tests SKIP no-rlib
		return
	fi

	local src name out compile_status
	for src in "$root"/examples/core-tests/*.rs; do
		[ -f "$src" ] || continue
		case "$(basename -- "$src")" in prelude.rs) continue ;; esac
		[ -f "${src%.rs}.expect_fail" ] && continue
		name=$(basename -- "${src%.rs}")
		out="$dir/$name.js"
		echo "==> $name (core)"
		JS_EXTRA_ARGS="${OPT_ARGS:+$OPT_ARGS }js-lane=shadow" SKIP_BUILD=1 with_timeout "$JSC_LANE_TIMEOUT" \
			"$script_dir/compile_core.sh" "$src" "$out" \
			--extern "mires_js=$rlib" --search "$macro_dir" \
			>"$log_dir/laneshadow-core-$name.compile.log" 2>&1
		compile_status=$?
		if [ "$compile_status" -eq 124 ]; then
			echo "    compile did not finish within ${JSC_LANE_TIMEOUT}s"
			record "core/$name" ERROR compile-timeout
			continue
		elif [ "$compile_status" -ne 0 ]; then
			echo "    compile failed, see $log_dir/laneshadow-core-$name.compile.log"
			record "core/$name" ERROR compile
			continue
		fi
		with_timeout "$JSC_LANE_TIMEOUT" run_script_mode "$out" \
			"$dir/$name.baseline.out" "$dir/$name.baseline.err"
		local baseline_status=$?
		with_timeout "$JSC_LANE_TIMEOUT" run_script_mode "$out.jir.js" \
			"$dir/$name.jir.out" "$dir/$name.jir.err"
		local jir_status=$?
		verdict "core/$name" "$out" "$out.jir.js" "$out.lane.json" \
			"$dir/$name.baseline.out" "$dir/$name.jir.out" "$baseline_status" "$jir_status"
	done
}

# -------------------------------------------------------------------- examples/dom-tests

run_dom_tests_suite() {
	echo "==> examples/dom-tests"
	if ! ensure_sysroot "$OPT_ARGS"; then
		echo "    skipped: no sysroot"
		record dom-tests SKIP no-sysroot
		return
	fi
	local dir="$build_dir/dom-tests"
	mkdir -p "$dir"
	# scripts/dom-test.sh's options, less the `js-names=readable` it spells out: readable names
	# are the default, so naming one changes nothing about the output and costs a sysroot of its
	# own. Same elision scripts/jir-coverage.sh makes, for the same reason.
	local dom_args="js-comments=on js-modules=esm js-dom-module=./topcoat-dom.js"

	if ! (cd "$root" && cargo build -p view-dom-macro) >"$log_dir/laneshadow-dom-macro.log" 2>&1; then
		echo "    skipped: view-dom-macro did not build"
		record dom-tests SKIP no-macro
		return
	fi
	local proc_macro="$root/target/debug/libview_dom_macro.$macro_suffix"
	local abi="$dir/libview_abi.rlib"
	if ! JS_EXTRA_ARGS="$dom_args${OPT_ARGS:+ $OPT_ARGS}" SKIP_BUILD=1 "$script_dir/compile_core.sh" \
		"$root/view-abi/src/lib.rs" "$abi" --crate-type rlib \
		>"$log_dir/laneshadow-dom-abi.log" 2>&1; then
		echo "    skipped: view-abi did not build"
		record dom-tests SKIP no-rlib
		return
	fi

	# The run directory dom-trace.mjs needs beside the program: a module package, the ES module
	# shim, and the DOM runtime, which is the contract's recording stub. Same layout as
	# scripts/dom-test.sh writes; a compiled program resolves both by relative specifier.
	printf '{ "type": "module" }\n' >"$dir/package.json"
	if ! node "$script_dir/make-esm-shim.mjs" "$dir/shim.js" >/dev/null; then
		echo "    skipped: could not derive the ES module shim"
		record dom-tests SKIP no-shim
		return
	fi
	cat >"$dir/topcoat-dom.js" <<'EOF'
// The DOM runtime a compiled client program imports; written by scripts/lane-shadow-test.sh and
// identical to the one scripts/dom-test.sh writes, so a trace taken here is that suite's trace.
export * from "../../../contract/harness/trace.mjs";

export function createSignal(initial) {
  let value = initial;
  return [
    () => value,
    (next) => {
      value = typeof next === "function" ? next(value) : next;
      return value;
    }
  ];
}
EOF

	local src name out compile_status
	for src in "$root"/examples/dom-tests/*.rs; do
		[ -f "$src" ] || continue
		name=$(basename -- "${src%.rs}")
		out="$dir/$name.js"
		echo "==> $name (dom)"
		JS_EXTRA_ARGS="$dom_args${OPT_ARGS:+ $OPT_ARGS} js-lane=shadow" SKIP_BUILD=1 with_timeout "$JSC_LANE_TIMEOUT" \
			"$script_dir/compile_core.sh" "$src" "$out" \
			--extern "view_abi=$abi" --extern "view_dom_macro=$proc_macro" \
			>"$log_dir/laneshadow-dom-$name.compile.log" 2>&1
		compile_status=$?
		if [ "$compile_status" -eq 124 ]; then
			echo "    compile did not finish within ${JSC_LANE_TIMEOUT}s"
			record "dom/$name" ERROR compile-timeout
			continue
		elif [ "$compile_status" -ne 0 ]; then
			echo "    compile failed, see $log_dir/laneshadow-dom-$name.compile.log"
			record "dom/$name" ERROR compile
			continue
		fi
		with_timeout "$JSC_LANE_TIMEOUT" node "$script_dir/dom-trace.mjs" "$out" \
			"laneshadow/$name" >"$dir/$name.baseline.out" 2>"$dir/$name.baseline.err"
		local baseline_status=$?
		with_timeout "$JSC_LANE_TIMEOUT" node "$script_dir/dom-trace.mjs" "$out.jir.js" \
			"laneshadow/$name" >"$dir/$name.jir.out" 2>"$dir/$name.jir.err"
		local jir_status=$?
		verdict "dom/$name" "$out" "$out.jir.js" "$out.lane.json" \
			"$dir/$name.baseline.out" "$dir/$name.jir.out" "$baseline_status" "$jir_status"
	done
}

# ------------------------------------------------------------------ examples/async-tests

# normalize_behaviour <in> <out>: the executor's check log with the two values that move between
# two compilations replaced.
#
# Fixture 06 drives the framework's real `#[procedure]` expansion, which mints a fresh UUID per
# expansion and routes the request to it. Two compilations therefore disagree about it whatever
# lane they were printed from, so comparing the raw log would report a difference that is not one.
# Everything else in the log — the order of the steps, which task resumed when — is the subject.
#
# The `shape:` checks (async-check.mjs) quote the emitted line they matched as their detail, and
# an emitted line names lane-dependent locals, so two lanes disagree about the quotation while
# agreeing about the shape. The detail tail after the label is stripped — the `ok`/`FAIL` verdict
# and the label are the columns compared (Phase 5, D10e). Only `shape:` lines are touched: a
# behaviour check's detail is a step sequence, and those must still agree verbatim.
normalize_behaviour() {
	sed -E -e 's/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/<uuid>/g' \
		-e 's/^((ok  |FAIL)  shape: [^:]*):.*/\1/' "$1" >"$2"
}

run_async_tests_suite() {
	echo "==> examples/async-tests (the whole suite twice; see the header)"
	# scripts/async-test.sh spells out `js-names=readable`, which keys a sysroot of its own.
	if ! ensure_sysroot "js-names=readable${OPT_ARGS:+ $OPT_ARGS}"; then
		echo "    skipped: no sysroot"
		record async-tests SKIP no-sysroot
		return
	fi
	local behaviour="$log_dir/async-behaviour.log"
	if ! SKIP_BUILD=1 JS_EXTRA_ARGS="$OPT_ARGS" ASYNC_OPT_MODE="$SHADOW_OPT_MODE" "$script_dir/async-test.sh" \
		>"$log_dir/laneshadow-async-baseline.log" 2>&1; then
		echo "    skipped: the suite does not pass on the baseline lane"
		record async-tests SKIP baseline-failed
		return
	fi
	normalize_behaviour "$behaviour" "$log_dir/laneshadow-async-baseline.behaviour"
	if ! SKIP_BUILD=1 JS_EXTRA_ARGS="${OPT_ARGS:+$OPT_ARGS }js-lane=jir" ASYNC_OPT_MODE="$SHADOW_OPT_MODE" "$script_dir/async-test.sh" \
		>"$log_dir/laneshadow-async-jir.log" 2>&1; then
		echo "    the suite fails on the JIR lane, see $log_dir/laneshadow-async-jir.log"
		record async-tests DIFF suite-failed
		return
	fi
	normalize_behaviour "$behaviour" "$log_dir/laneshadow-async-jir.behaviour"
	# The sidecars the JIR run left behind, so the histogram counts this suite too.
	mkdir -p "$build_dir/async-tests"
	cp "$root"/build/asynctest/*.lane.json "$build_dir/async-tests/" 2>/dev/null

	if ! diff -u "$log_dir/laneshadow-async-baseline.behaviour" \
		"$log_dir/laneshadow-async-jir.behaviour" >"$log_dir/laneshadow-async.diff"; then
		echo "    the executor behaved differently (- baseline, + jir):"
		sed 's/^/    /' "$log_dir/laneshadow-async.diff" | head -n 40
		record async-tests DIFF behaviour
		return
	fi
	local lowered=0 fallbacks=0 sidecar
	for sidecar in "$build_dir"/async-tests/*.lane.json; do
		[ -f "$sidecar" ] || continue
		lowered=$((lowered + $(lane_count "$sidecar" jir)))
		fallbacks=$((fallbacks + $(lane_count "$sidecar" fallback)))
	done
	if [ "$lowered" -eq 0 ]; then
		echo "    ok (no item lowered, behaviour identical)"
		record async-tests FALLBACK-ONLY "$fallbacks fallbacks"
	else
		echo "    ok ($lowered items lowered, behaviour identical)"
		record async-tests PASS "$lowered lowered, $fallbacks fallbacks"
	fi
}

for suite in $suites; do
	case "$suite" in
	tests) run_tests_suite ;;
	core-tests) run_core_tests_suite ;;
	dom-tests) run_dom_tests_suite ;;
	async-tests) run_async_tests_suite ;;
	esac
done

# --------------------------------------------------------------------------- the report

echo
echo "================================================================"
printf '%-28s %-15s %s\n' "FIXTURE" "RESULT" "DETAIL"
echo "----------------------------------------------------------------"
printf '%s' "$results" | while read -r name result detail; do
	printf '%-28s %-15s %s\n' "$name" "$result" "$detail"
done
echo "----------------------------------------------------------------"
printf '%d/%d agree\n' "$((total - failures))" "$total"
echo "================================================================"

# The worklist: which declines and which classes the fallbacks came from, over every sidecar.
node - "$build_dir" <<'HISTOGRAM'
const fs = require("node:fs")
const path = require("node:path")

const root = process.argv[2]
const files = []
const walk = (dir) => {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name)
    if (entry.isDirectory()) walk(full)
    else if (entry.name.endsWith(".lane.json")) files.push(full)
  }
}
if (fs.existsSync(root)) walk(root)

const declines = new Map()
const classes = new Map()
let lowered = 0
let fallbacks = 0
for (const file of files) {
  let doc
  try {
    doc = JSON.parse(fs.readFileSync(file, "utf8"))
  } catch {
    continue
  }
  for (const item of doc.items ?? []) {
    if (item.lane === "jir") {
      lowered += 1
      continue
    }
    fallbacks += 1
    const code = item.decline ?? "(none)"
    const seen = declines.get(code) ?? { items: 0, bytes: 0 }
    seen.items += 1
    seen.bytes += item.bytes_baseline ?? 0
    declines.set(code, seen)
    for (const name of item.class.length > 0 ? item.class : ["(none)"]) {
      const byClass = classes.get(name) ?? { items: 0, bytes: 0 }
      byClass.items += 1
      byClass.bytes += item.bytes_baseline ?? 0
      classes.set(name, byClass)
    }
  }
}

const pad = (value, width) => String(value).padStart(width)
console.log("")
console.log(`FALLBACKS over ${files.length} sidecars: ${lowered} items lowered, ${fallbacks} fell back`)
const table = (title, map) => {
  if (map.size === 0) return
  console.log("----------------------------------------------------------------")
  console.log(title)
  for (const [key, counts] of [...map].sort((a, b) => b[1].bytes - a[1].bytes)) {
    console.log(`${pad(counts.bytes, 10)} B  ${pad(counts.items, 6)} items  ${key}`)
  }
}
table("by decline code", declines)
table("by item class", classes)
console.log("================================================================")
HISTOGRAM

if [ "$failures" -ne 0 ]; then
	exit 1
fi
