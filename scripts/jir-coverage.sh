#!/usr/bin/env bash
#
# How much of a program the JIR lane could print, measured over the fixtures and over the
# FingerprintJS ports.
#
#   scripts/jir-coverage.sh                       # every fixture suite
#   scripts/jir-coverage.sh --fixtures
#   scripts/jir-coverage.sh --fingerprintjs
#   scripts/jir-coverage.sh --fixtures --sysroot-args 'js-minify=on'
#
# The number this exists to produce is not "how many items have a JIR body" but "how many BYTES of
# the emitted program a lane could own", which is the only form of the question a size gate can be
# written against: one 40KB item that has to fall back to the printed baseline costs more than
# forty small ones that do not.
#
# WHERE THE NUMBERS COME FROM
# ---------------------------
# The authoritative source is `-Cllvm-args=js-lane-report=<path>`, the link step's per item lane
# report: `{items: [{name, bytes, availability, decline, class, lane}]}`. Both halves of this
# script read it: the fixtures ask for it directly, and the FingerprintJS ports ask their own
# build script for it with `--lane-report`, which publishes it beside the bundles. The optimizer
# report's `items[].late_ir_analysis` remains as a fallback for a port or a fixture that produced
# no lane report — it answers the same question in the v1 spelling, `status` is `available`,
# `semantic` or `unavailable` and `fallback` is the reason — and the SOURCE column says which
# file each suite's numbers came from, so a silent downgrade is visible.
#
# `js-opt=report` is passed to every compilation for the fallback's sake: the late IR analysis only
# runs when the optimizer is at least reporting, and with it off every item reads as unavailable
# for a reason that has nothing to do with the lane. It is emit-affecting, so the suites compiled
# against the real `core` need a sysroot built the same way; this script builds one if it is
# missing (it does NOT rebuild the backend — the sysroot is keyed on the dylib that is already
# there, and rebuilding it mid-run would key the sysroot to a different compiler).
#
# WHAT IS MEASURED
# ----------------
# Every fixture that compiles is one program, measured whole. A fixture that does not compile is
# reported as skipped rather than silently dropped, and a suite whose scaffolding cannot be built
# (a proc macro, an rlib) is skipped with its reason: this is a measurement tool, and a suite
# missing from the total has to be visible in the output.

set -uo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
# shellcheck source=sysroot-cache.sh
source "$script_dir/sysroot-cache.sh"

build_dir="$root/build/jir-coverage"
log_dir="$root/build/logs"
manifest="$build_dir/manifest.tsv"

usage() {
	cat >&2 <<'USAGE'
usage: jir-coverage.sh [--fixtures] [--fingerprintjs] [--sysroot-args '<backend options>']

  --fixtures        measure examples/{tests,core-tests,dom-tests,async-tests,native-tests}
  --fingerprintjs   measure the FingerprintJS idiomatic and compact ports
  --sysroot-args    extra backend options for every compilation (and so for the sysroot)

With neither --fixtures nor --fingerprintjs, --fixtures is assumed.
USAGE
}

want_fixtures=0
want_fingerprintjs=0
extra_args=""
while [ "$#" -gt 0 ]; do
	case "$1" in
	--fixtures)
		want_fixtures=1
		shift
		;;
	--fingerprintjs)
		want_fingerprintjs=1
		shift
		;;
	--sysroot-args)
		if [ "$#" -lt 2 ]; then
			echo "jir-coverage.sh: --sysroot-args needs a value" >&2
			exit 2
		fi
		extra_args=$2
		shift 2
		;;
	-h | --help)
		usage
		exit 0
		;;
	*)
		echo "jir-coverage.sh: unknown option: $1" >&2
		usage
		exit 2
		;;
	esac
done
if [ "$want_fixtures" -eq 0 ] && [ "$want_fingerprintjs" -eq 0 ]; then
	want_fixtures=1
fi

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
if [ ! -f "$dylib" ]; then
	echo "jir-coverage.sh: backend not built: $dylib" >&2
	echo "jir-coverage.sh: run scripts/build.sh first" >&2
	exit 1
fi

# The options every compilation gets. The user's own come last so that any of these can be
# overridden — `--sysroot-args 'js-jir=off'` is how the measurement is checked against a compiler
# that carries no JIR bodies at all.
base_args="js-opt=report js-opt-warnings=off $extra_args"

rm -rf "$build_dir"
mkdir -p "$build_dir" "$log_dir"
: >"$manifest"

# record <suite> <name> <state> <lane report> <opt report>
record() {
	printf '%s\t%s\t%s\t%s\t%s\n' "$1" "$2" "$3" "${4:-}" "${5:-}" >>"$manifest"
}

# ------------------------------------------------------------------ the sysroot
#
# Only the suites compiled against the real `core` need one, so it is resolved lazily: a
# `--fingerprintjs`-only run drives the ports' own build scripts, which resolve their own.

sysroot_reason=""

# ensure_sysroot <backend options>: the keyed sysroot those options need, built if it is absent.
#
# A suite that passes emit-affecting options of its own needs a sysroot of its own, so this is
# asked per suite rather than once. Each answer is remembered by its normalized key, which is what
# keeps the second suite with the same options from paying for the check twice.
sysroot_checked=""

ensure_sysroot() {
	local args path
	args=$(jsc_normalize_emit_args "${1:-}")
	case " $sysroot_checked " in
	*"|$args|"*)
		[ -z "$sysroot_reason" ]
		return
		;;
	esac
	path=$(JSC_SYSROOT_ARGS="$args" jsc_sysroot_path "$root" wasm32-unknown-unknown "$dylib") || {
		sysroot_reason="the keyed sysroot cannot be resolved"
		return 1
	}
	if [ ! -d "$path/lib/rustlib/wasm32-unknown-unknown/lib" ]; then
		echo "==> building the sysroot for '$args' (about a minute)"
		if ! SKIP_BUILD=1 JS_EXTRA_ARGS="$args" "$script_dir/build_sysroot.sh" \
			>"$log_dir/jir-coverage-sysroot.log" 2>&1; then
			sysroot_reason="the sysroot build failed, see $log_dir/jir-coverage-sysroot.log"
			return 1
		fi
	fi
	sysroot_checked="$sysroot_checked |$args|"
	sysroot_reason=""
	return 0
}

# The `mires-js` rlib and the proc macro it needs, offered to the suites that may name them.
# `--extern` only puts a crate on the search path, so a fixture that never names it links nothing.
core_externs=()
mires_js_prepared=0

prepare_mires_js() {
	[ "$mires_js_prepared" -eq 1 ] && return
	mires_js_prepared=1
	local macro_dir="$build_dir/core-macros"
	mkdir -p "$macro_dir"
	if ! (cd "$root" && cargo build -p js-extern-macro) \
		>"$log_dir/jir-coverage-core-macro.log" 2>&1; then
		echo "    note: js-extern-macro did not build; the fixtures that name mires-js will fail"
		return
	fi
	cp "$root/target/debug/libjs_extern_macro.$macro_suffix" "$macro_dir/"
	local rlib="$build_dir/libmires_js.rlib"
	if ! JS_EXTRA_ARGS="$(jsc_normalize_emit_args "$base_args")" "$script_dir/compile_core.sh" \
		"$root/mires-js/src/lib.rs" "$rlib" --crate-type rlib --crate-name mires_js \
		--extern "js_extern_macro=$root/target/debug/libjs_extern_macro.$macro_suffix" \
		>"$log_dir/jir-coverage-core-mires-js.log" 2>&1; then
		echo "    note: mires-js did not build; the fixtures that name it will fail"
		return
	fi
	core_externs=(--extern "mires_js=$rlib" --search "$macro_dir")
}

# ------------------------------------------------------------------- one fixture
#
# compile_fixture <suite> <compiler> <source> <extra compiler arguments...>

compile_fixture() {
	local suite=$1 compiler=$2 src=$3
	shift 3
	local name out lane opt
	name=$(basename -- "${src%.rs}")
	out="$build_dir/$suite/$name.js"
	lane="$build_dir/$suite/$name.lane.json"
	opt="$build_dir/$suite/$name.opt.json"
	mkdir -p "$build_dir/$suite"

	if JS_EXTRA_ARGS="$base_args $suite_args js-opt-report=$opt js-lane-report=$lane" \
		"$script_dir/$compiler" "$src" "$out" "$@" \
		>"$log_dir/jir-coverage-$suite-$name.log" 2>&1; then
		record "$suite" "$name" compiled "$lane" "$opt"
	else
		record "$suite" "$name" failed "" ""
	fi
}

# ------------------------------------------------------------------- the suites

suite_args=""

measure_tests() {
	echo "==> examples/tests (no_core, scripts/compile.sh)"
	suite_args=""
	local src
	for src in "$root"/examples/tests/*.rs; do
		[ -f "$src" ] || continue
		# The expected-failure fixtures are compiler diagnostics, not programs.
		[ -f "${src%.rs}.expect_fail" ] && continue
		compile_fixture tests compile.sh "$src"
	done
}

measure_core_tests() {
	echo "==> examples/core-tests (real core, scripts/compile_core.sh)"
	suite_args=""
	if ! ensure_sysroot "$base_args"; then
		echo "    skipped: $sysroot_reason"
		record core-tests '(suite)' skipped-no-sysroot "" ""
		return
	fi
	prepare_mires_js

	local src
	for src in "$root"/examples/core-tests/*.rs; do
		[ -f "$src" ] || continue
		case "$(basename -- "$src")" in
		prelude.rs) continue ;;
		esac
		[ -f "${src%.rs}.expect_fail" ] && continue
		compile_fixture core-tests compile_core.sh "$src" ${core_externs[@]+"${core_externs[@]}"}
	done
}

measure_native_tests() {
	echo "==> examples/native-tests"
	suite_args=""
	if ! ensure_sysroot "$base_args"; then
		echo "    skipped: $sysroot_reason"
		record native-tests '(suite)' skipped-no-sysroot "" ""
		return
	fi
	# The native fixtures are written against the host collections crate, as the core ones are.
	prepare_mires_js
	local src
	for src in "$root"/examples/native-tests/*.rs; do
		[ -f "$src" ] || continue
		# This directory also holds the ORACLES the differential scripts run: ordinary `std`
		# programs compiled by the host rustc and compared against the emitted JavaScript. They
		# are not this backend's input and would only ever be counted as failures.
		grep -q '^#!\[no_std\]' "$src" || continue
		compile_fixture native-tests compile_core.sh "$src" ${core_externs[@]+"${core_externs[@]}"}
	done
}

measure_dom_tests() {
	echo "==> examples/dom-tests"
	# The options scripts/dom-test.sh compiles its fixtures with, less the `js-names=readable`
	# it spells out: readable names are the default, and naming a default costs a sysroot of its
	# own here for output that is identical either way.
	suite_args="js-comments=on js-modules=esm js-dom-module=./topcoat-dom.js"
	if ! ensure_sysroot "$base_args $suite_args"; then
		echo "    skipped: $sysroot_reason"
		record dom-tests '(suite)' skipped-no-sysroot "" ""
		return
	fi

	if ! (cd "$root" && cargo build -p view-dom-macro) >"$log_dir/jir-coverage-dom-macro.log" 2>&1; then
		echo "    skipped: view-dom-macro did not build, see $log_dir/jir-coverage-dom-macro.log"
		record dom-tests '(suite)' skipped-no-macro "" ""
		return
	fi
	local proc_macro="$root/target/debug/libview_dom_macro.$macro_suffix"
	local abi="$build_dir/libview_abi.rlib"
	if ! JS_EXTRA_ARGS="$base_args $suite_args" "$script_dir/compile_core.sh" \
		"$root/view-abi/src/lib.rs" "$abi" --crate-type rlib --crate-name view_abi \
		>"$log_dir/jir-coverage-dom-abi.log" 2>&1; then
		echo "    skipped: view-abi did not build, see $log_dir/jir-coverage-dom-abi.log"
		record dom-tests '(suite)' skipped-no-abi "" ""
		return
	fi

	local src
	for src in "$root"/examples/dom-tests/*.rs; do
		[ -f "$src" ] || continue
		compile_fixture dom-tests compile_core.sh "$src" \
			--extern "view_abi=$abi" --extern "view_dom_macro=$proc_macro"
	done
}

measure_async_tests() {
	echo "==> examples/async-tests"
	suite_args="js-comments=on js-modules=esm js-dom-module=./topcoat-dom.js"
	if ! ensure_sysroot "$base_args $suite_args"; then
		echo "    skipped: $sysroot_reason"
		record async-tests '(suite)' skipped-no-sysroot "" ""
		return
	fi

	# The scaffolding scripts/async-test.sh builds, in the same order: three host proc macros (one
	# of them out of the framework workspace, with the spike's pinned nightly) and three rlibs
	# compiled by this backend. Any of them failing skips the suite with the reason rather than
	# reporting a coverage number the fixtures did not contribute to.
	local channel
	channel=$(jsc_pinned_channel "$root")
	if ! (cd "$root" && cargo build -p js-macro -p js-extern-macro) \
		>"$log_dir/jir-coverage-async-macros.log" 2>&1; then
		echo "    skipped: the proc macros did not build, see $log_dir/jir-coverage-async-macros.log"
		record async-tests '(suite)' skipped-no-macro "" ""
		return
	fi
	if ! (cd "$root/../.." && cargo "+$channel" build -p topcoat-runtime-macro --features discover \
		--target-dir "$root/build/hostmacro-discover") \
		>"$log_dir/jir-coverage-async-procedure.log" 2>&1; then
		echo "    skipped: topcoat-runtime-macro did not build, see $log_dir/jir-coverage-async-procedure.log"
		record async-tests '(suite)' skipped-no-macro "" ""
		return
	fi
	local js_macro="$root/target/debug/libjs_macro.$macro_suffix"
	local extern_macro="$root/target/debug/libjs_extern_macro.$macro_suffix"
	local procedure_macro="$root/build/hostmacro-discover/debug/libtopcoat_runtime_macro.$macro_suffix"
	# A directory of proc macros and nothing else: rustc finds a transitive dependency by file
	# name on the search path, and an rlib built for the host beside them reports the wrong triple.
	local macro_dir="$build_dir/async-macros"
	mkdir -p "$macro_dir"
	cp "$js_macro" "$extern_macro" "$macro_dir/"

	local abi="$build_dir/libview_abi_async.rlib"
	local async_rlib="$build_dir/libview_async.rlib"
	local dom_rlib="$build_dir/libtopcoat_dom.rlib"
	local step
	for step in \
		"view_abi|$root/view-abi/src/lib.rs|$abi" \
		"view_async|$root/view-async/src/lib.rs|$async_rlib" \
		"topcoat_dom|$root/topcoat-dom/src/lib.rs|$dom_rlib"; do
		local crate=${step%%|*}
		local rest=${step#*|}
		local source=${rest%%|*}
		local target=${rest#*|}
		if ! JS_EXTRA_ARGS="$base_args $suite_args" "$script_dir/compile_core.sh" \
			"$source" "$target" --crate-type rlib --crate-name "$crate" \
			--search "$build_dir" --search "$macro_dir" \
			--extern "view_abi=$abi" --extern "view_async=$async_rlib" \
			--extern "js_macro=$js_macro" \
			>"$log_dir/jir-coverage-async-$crate.log" 2>&1; then
			echo "    skipped: $crate did not build, see $log_dir/jir-coverage-async-$crate.log"
			record async-tests '(suite)' skipped-no-rlib "" ""
			return
		fi
	done

	local src
	for src in "$root"/examples/async-tests/*.rs; do
		[ -f "$src" ] || continue
		case "$(basename -- "$src")" in
		prelude.rs) continue ;;
		esac
		compile_fixture async-tests compile_core.sh "$src" \
			--cfg topcoat_client --search "$build_dir" --search "$macro_dir" \
			--extern "view_abi=$abi" --extern "view_async=$async_rlib" \
			--extern "topcoat_dom=$dom_rlib" --extern "js_macro=$js_macro" \
			--extern "js_extern_macro=$extern_macro" \
			--extern "topcoat_runtime_macro=$procedure_macro"
	done
}

# ----------------------------------------------------------- the FingerprintJS ports
#
# Both ports build themselves, through their own `tools/build.mjs`, against this checkout, and are
# asked for the link step's lane report with `--lane-report`. That is the same file the fixture
# half reads, so both halves of this measurement now come from the compiler's own answer rather
# than from the optimizer report's restatement of it.

measure_fingerprintjs() {
	local repository
	repository=$(cd -- "$root/../.." && pwd)
	local port
	for port in fingerprintjs-rust fingerprintjs-rust-compact; do
		local dir="$repository/experiments/$port"
		echo "==> $port"
		if [ ! -f "$dir/tools/build.mjs" ]; then
			echo "    skipped: no build script at $dir/tools/build.mjs"
			record "$port" '(port)' skipped-no-build "" ""
			continue
		fi
		if ! (cd "$dir" && node tools/build.mjs --compiler-root "$root" --optimizer report \
			--lane-report) \
			>"$log_dir/jir-coverage-$port.log" 2>&1; then
			echo "    skipped: the build failed, see $log_dir/jir-coverage-$port.log"
			record "$port" '(port)' failed "" ""
			continue
		fi
		# Only the build this run just made. `tools/build.mjs` names its build directory after the
		# mode — which is more than the optimizer setting, the compact port defaults to
		# `-mir-inline` as well — and suffixes it with the compiler checkout's directory name
		# whenever that is not the frozen v1 default. So the suffix is what identifies this run's
		# trees, and a `report` tree left behind by v1 is a different program that must not be
		# counted beside them.
		local suffix="-$(basename -- "$root")"
		local reports=() report
		for report in "$dir"/target/js-build/report*"$suffix"/*/optimizer-report.json; do
			[ -f "$report" ] && reports+=("$report")
		done
		if [ "${#reports[@]}" -eq 0 ]; then
			for report in "$dir"/target/js-build/report*/*/optimizer-report.json; do
				[ -f "$report" ] && reports+=("$report")
			done
		fi
		local found=0
		for report in ${reports[@]+"${reports[@]}"}; do
			local variant lane
			variant=$(basename -- "$(dirname -- "$report")")
			# The lane report the build was asked for, beside the optimizer report it also
			# writes. Recorded as the first source, so the summariser prefers it.
			lane="$(dirname -- "$report")/lane-report.json"
			[ -f "$lane" ] || lane=""
			record "$port" "$variant" compiled "$lane" "$report"
			found=1
		done
		if [ "$found" -eq 0 ]; then
			echo "    skipped: the build wrote no optimizer report"
			record "$port" '(port)' no-report "" ""
		fi
	done
}

if [ "$want_fixtures" -eq 1 ]; then
	measure_tests
	measure_core_tests
	measure_native_tests
	measure_dom_tests
	measure_async_tests
fi
if [ "$want_fingerprintjs" -eq 1 ]; then
	measure_fingerprintjs
fi

# ------------------------------------------------------------------ the summary

cat >"$build_dir/summarize.mjs" <<'SUMMARY'
// Reads the manifest jir-coverage.sh wrote and prints what the lane could own.
//
// Two report shapes are understood, and which one a fixture contributed is part of the output:
//   * the lane report, `{items: [{name, bytes, availability, decline, class}]}`;
//   * the optimizer report, whose `items[].late_ir_analysis.{status,fallback}` answers the same
//     question in the v1 spelling.
import { readFileSync, existsSync } from 'node:fs'

const manifest = process.argv[2]
const rows = readFileSync(manifest, 'utf8').split('\n').filter(Boolean).map((line) => {
  const [suite, name, state, lane, opt] = line.split('\t')
  return { suite, name, state, lane, opt }
})

const AVAILABILITY = ['available', 'semantic', 'unavailable']
const empty = () => ({
  fixtures: 0, failed: 0, skipped: 0,
  items: Object.fromEntries(AVAILABILITY.map((a) => [a, 0])),
  bytes: Object.fromEntries(AVAILABILITY.map((a) => [a, 0])),
  sources: { lane: 0, 'opt-fallback': 0, none: 0 },
})
const suites = new Map()
const declines = new Map()
const suiteOf = (name) => {
  if (!suites.has(name)) suites.set(name, empty())
  return suites.get(name)
}

// The lane report's availability vocabulary is the same three words; anything else is counted as
// unavailable rather than dropped, so a vocabulary that grows shows up as a number that moves.
const normalize = (value) => (AVAILABILITY.includes(value) ? value : 'unavailable')

for (const row of rows) {
  const suite = suiteOf(row.suite)
  if (row.state !== 'compiled') {
    if (row.state === 'failed') suite.failed += 1
    else suite.skipped += 1
    continue
  }
  suite.fixtures += 1

  let items = null
  let source = 'none'
  if (row.lane && existsSync(row.lane)) {
    const report = JSON.parse(readFileSync(row.lane, 'utf8'))
    items = (report.items ?? []).map((item) => ({
      bytes: item.bytes ?? 0,
      availability: normalize(item.availability),
      decline: item.decline ?? null,
    }))
    source = 'lane'
  } else if (row.opt && existsSync(row.opt)) {
    const report = JSON.parse(readFileSync(row.opt, 'utf8'))
    items = (report.items ?? []).map((item) => ({
      bytes: item.bytes ?? 0,
      availability: normalize(item.late_ir_analysis?.status),
      decline: item.late_ir_analysis?.fallback ?? null,
    }))
    source = 'opt-fallback'
  }
  suite.sources[source] += 1
  if (!items) continue

  for (const item of items) {
    suite.items[item.availability] += 1
    suite.bytes[item.availability] += item.bytes
    if (item.availability === 'available' || !item.decline) continue
    const seen = declines.get(item.decline) ?? { items: 0, bytes: 0 }
    seen.items += 1
    seen.bytes += item.bytes
    declines.set(item.decline, seen)
  }
}

const total = empty()
for (const suite of suites.values()) {
  total.fixtures += suite.fixtures
  total.failed += suite.failed
  total.skipped += suite.skipped
  for (const key of AVAILABILITY) {
    total.items[key] += suite.items[key]
    total.bytes[key] += suite.bytes[key]
  }
  for (const key of Object.keys(total.sources)) total.sources[key] += suite.sources[key]
}

const pct = (part, whole) => (whole === 0 ? '   -  ' : `${((part / whole) * 100).toFixed(1)}%`.padStart(6))
const num = (value, width) => String(value).padStart(width)
const bytesOf = (counts) => AVAILABILITY.reduce((sum, key) => sum + counts[key], 0)

console.log('')
console.log('================================================================================')
console.log('JIR coverage: how many emitted bytes a lane could own')
console.log('--------------------------------------------------------------------------------')
console.log(
  'SUITE'.padEnd(16), 'FIX'.padStart(4), 'ITEMS'.padStart(6), 'AVAIL'.padStart(6),
  'BYTES'.padStart(9), 'AVAIL'.padStart(9), 'AVAIL%'.padStart(7), 'SOURCE'.padStart(13),
)
const line = (name, counts) => {
  const items = AVAILABILITY.reduce((sum, key) => sum + counts.items[key], 0)
  const bytes = bytesOf(counts.bytes)
  const source = counts.sources.lane > 0
    ? (counts.sources['opt-fallback'] > 0 ? 'mixed' : 'lane-report')
    : (counts.sources['opt-fallback'] > 0 ? 'opt-fallback' : '-')
  console.log(
    name.padEnd(16), num(counts.fixtures, 4), num(items, 6), num(counts.items.available, 6),
    num(bytes, 9), num(counts.bytes.available, 9), pct(counts.bytes.available, bytes).padStart(7),
    source.padStart(13),
  )
}
for (const [name, counts] of suites) line(name, counts)
console.log('--------------------------------------------------------------------------------')
line('TOTAL', total)
console.log('')
console.log(
  `items: ${total.items.available} available, ${total.items.semantic} semantic, ` +
  `${total.items.unavailable} unavailable`,
)
console.log(
  `bytes: ${total.bytes.available} available, ${total.bytes.semantic} semantic, ` +
  `${total.bytes.unavailable} unavailable`,
)
const skipped = [...suites].filter(([, counts]) => counts.failed || counts.skipped)
for (const [name, counts] of skipped) {
  console.log(`note: ${name}: ${counts.failed} fixture(s) failed to compile, ${counts.skipped} skipped`)
}

if (declines.size > 0) {
  console.log('')
  console.log('DECLINES (why a body was not available), by bytes')
  console.log('--------------------------------------------------------------------------------')
  const sorted = [...declines].sort((a, b) => b[1].bytes - a[1].bytes)
  for (const [reason, counts] of sorted) {
    console.log(`${num(counts.bytes, 9)}  ${num(counts.items, 5)} items  ${reason}`)
  }
}
console.log('================================================================================')
SUMMARY

node "$build_dir/summarize.mjs" "$manifest"
