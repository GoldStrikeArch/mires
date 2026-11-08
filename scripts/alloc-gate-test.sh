#!/usr/bin/env bash
#
# The site gate: the measure the optimizer's claim is actually made in.
#
#   scripts/alloc-gate-test.sh [FIXTURE...]
#   scripts/alloc-gate-test.sh --fingerprintjs
#
# WHAT IS COUNTED, AND WHAT IT IS NOT
# -----------------------------------
# Not allocations at run time — nothing static can know those. What is counted is
# the number of PLACES the emitted JavaScript builds a fresh object or array, and
# `alloc` is the JIR pressure accounting retained for pass diagnostics. The authoritative
# performance gate reads `js_alloc`, a walk of the final lowered JavaScript AST after carriers,
# adapters, fusion, and lowering; it therefore sees record-shaped views and overflow-result
# arrays that JIR itself did not allocate.
#
# The first form compiles the virtual-aggregate fixtures on the JIR lane with the
# optimizer on and rustc's MIR inliner on, and compares the program's site totals
# — the `alloc` block of every row of `<out>.lane.json`, summed — against the
# fixture's `.alloc` file, exactly. That file is one line of five counts:
#
#   materialize=<n> copy=<n> closure=<n> view=<n> in_loop=<n>
#
# `view` counts the references, raw pointers and slice views whose pointer form is
# a record (`{ buf, off }`, `{ buf, off, len }`, `{ ptr, meta }`), which print a
# fresh object exactly like an aggregate does. It is REPORTED and not gated: a
# reference carrier is not an escape-analysis candidate by design, so a gate on it
# would pin a decision rather than catch a regression.
#
# `alloc`, the global allocator's own calls, is printed but not compared: how
# often `core` reaches the allocator is not a fact this phase owns. With no
# FIXTURE arguments every `examples/core-tests/*.alloc` is checked; naming one
# checks only it (with or without the `.rs`). `ALLOC_WRITE=1` writes each named
# fixture's `.alloc` from what it measures instead of comparing, which is how the
# file is made in the first place and remade when a pass legitimately changes it.
#
# Why counts and not bytes: scalar replacement trades an object literal for a few
# more locals, so the byte count of a function can go either way, and a `{ high,
# low }` built once per byte of input is invisible to every measure except this
# one.
#
# The second form is the Murmur gate. It needs the FingerprintJS experiments
# beside this checkout, builds the port with this compiler on the JIR lane with
# the optimizer on, and reports four numbers:
#
#   1. final JavaScript object/array/closure/`new` sites inside the hashing kernel's loops. The
#      current North Star gate is exactly zero, and opaque raw JavaScript inside a loop also fails because
#      it is not proof of allocation freedom. JIR materialize/copy/view sites remain printed as
#      diagnostics.
#   2. size: those items' `bytes_min` (each printed compact with its locals shortened whatever the
#      build's own `js-minify` setting is, so a size claim about one item does not need a
#      whole-program minify). The current North Star gate is at most 7,686 bytes, the frozen
#      Phase 6 attribution checkpoint. The minified ReScript module remains a diagnostic ratio.
#   3. speed: the geometric mean of the ASCII and Unicode p50 of the exported
#      `murmurX64Hash128` over the ReScript port's, three runs, best ratio, with
#      the hashes checked against the TypeScript oracle's. The gate is 1.05.
#   4. the ordinary idiomatic build (`mir-inline=off`) does not exceed its frozen post-slice
#      62,291-byte gzip-9 receipt. The Murmur kernel itself is measured from the separate
#      `mir-inline=on` build; comparing that package with the ordinary receipt mixes configurations.
#
# Nothing git tracks under `experiments/` is written: the port's build and dist
# trees are suffixed with this compiler's directory name.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
experiments=$(cd -- "$root/../.." && pwd)/experiments

# The gate is one number per fixture, so the totals live in one place: this reads
# a sidecar and prints the `.alloc` line, or the offending in-loop sites.
totals() {
	node - "$1" "${2:-}" <<'NODE'
const fs = require("node:fs");
const [, , sidecar, mode] = process.argv;
const report = JSON.parse(fs.readFileSync(sidecar, "utf8"));
if ((report.schema ?? 0) < 8) {
  throw new Error(`${sidecar} has schema ${report.schema ?? "missing"}; allocation provenance needs 8`);
}
const rows = report.items ?? [];
const totals = { materialize: 0, copy: 0, closure: 0, view: 0, alloc: 0, in_loop: 0 };
const sites = [];
let counted = 0;
for (const row of rows) {
  if (!row.alloc) continue;
  counted += 1;
  for (const key of Object.keys(totals)) totals[key] += row.alloc[key];
  for (const site of row.alloc.sites) {
    sites.push(`${row.name} ${site.site} ${site.kind} depth=${site.depth} ${site.origin}`);
  }
}
// A sidecar whose rows carry no `alloc` block would compare a row of zeroes
// against a row of zeroes and call it a pass. The analysis not having run is a
// failure.
if (counted === 0) {
  console.error(`${sidecar}: no row carries an alloc block (is the lane driver's hook wired?)`);
  process.exit(1);
}
if (mode === "--sites") {
  console.log(`alloc=${totals.alloc}`);
  for (const site of sites) console.log(site);
} else {
  const { materialize, copy, closure, view, in_loop: inLoop } = totals;
  console.log(
    `materialize=${materialize} copy=${copy} closure=${closure} view=${view} in_loop=${inLoop}`,
  );
}
NODE
}

run_fixtures() {
	local fixtures=("$@")
	if [ "${#fixtures[@]}" -eq 0 ]; then
		while IFS= read -r file; do
			fixtures+=("$(basename -- "${file%.alloc}")")
		done < <(find "$root/examples/core-tests" -name '*.alloc' | sort)
	fi
	if [ "${#fixtures[@]}" -eq 0 ]; then
		echo "alloc gate: no .alloc fixtures yet (60-66 land with the passes)"
		return 0
	fi

	JS_EXTRA_ARGS='js-opt=on' SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >/dev/null

	local work
	work=$(mktemp -d)
	# shellcheck disable=SC2064
	trap "rm -rf -- '$work'" RETURN

	local failed=0 fixture source_file expected_file out actual expected
	for fixture in "${fixtures[@]}"; do
		source_file="$root/examples/core-tests/$fixture.rs"
		expected_file="$root/examples/core-tests/$fixture.alloc"
		if [ ! -f "$source_file" ]; then
			echo "alloc gate: no such fixture: $source_file" >&2
			return 1
		fi
		if [ ! -f "$expected_file" ] && [ "${ALLOC_WRITE:-0}" != 1 ]; then
			echo "alloc gate: $fixture has no .alloc file (ALLOC_WRITE=1 writes one)" >&2
			return 1
		fi
		out="$work/$fixture.js"
		JSC_RUST_MIR_INLINE=on \
			JS_EXTRA_ARGS='js-lane=jir js-opt=on' \
			"$script_dir/compile_core.sh" "$source_file" "$out" >/dev/null
		if [ ! -f "$out.lane.json" ]; then
			echo "alloc gate: $fixture produced no sidecar at $out.lane.json" >&2
			return 1
		fi
		actual=$(totals "$out.lane.json")
		if [ "${ALLOC_WRITE:-0}" = 1 ]; then
			echo "$actual" >"$expected_file"
			echo "alloc gate $fixture: wrote $actual"
			continue
		fi
		expected=$(tr -d '\r' <"$expected_file" | head -n 1)
		if [ "$actual" = "$expected" ]; then
			echo "alloc gate $fixture: PASS ($actual)"
		else
			echo "alloc gate $fixture: FAIL"
			echo "  expected: $expected"
			echo "  actual:   $actual"
			totals "$out.lane.json" --sites | sed 's/^/  /'
			failed=1
		fi
	done
	return "$failed"
}

run_fingerprintjs() {
	local port="$experiments/fingerprintjs-rust"
	local rescript="$experiments/fingerprintjs-rescript"
	local oracle="$experiments/fingerprintjs/dist/fp.esm.js"
	local worker="$experiments/fingerprintjs-compare/tools/kernel-worker.mjs"
	local terser="$experiments/fingerprintjs/node_modules/.bin/terser"
	local path
	for path in "$port/tools/build.mjs" "$rescript/lib/es6/src/Hashing.res.mjs" \
		"$rescript/dist/fp.esm.min.js" "$oracle" "$worker" "$terser"; do
		if [ ! -e "$path" ]; then
			echo "alloc gate: required input is missing: $path" >&2
			return 1
		fi
	done

	# `--mir-inline on` is the port build's own flag and its default is off. The
	# escape analysis needs `X64::new` and the closure bodies inside their callers
	# before it can see an aggregate to scalarize at all.
	local mode=on-mir-inline-rustc-codegen-js-v2
	local package_mode=on-rustc-codegen-js-v2
	if [ "${SKIP_PORT_BUILD:-0}" != 1 ]; then
		(
			cd "$port" &&
				node tools/build.mjs \
					--compiler-root "$root" \
					--lane jir \
					--optimizer on \
					--mir-inline on \
					--lane-report \
					--scheduler boundary
		) >&2
		(
			cd "$port" &&
				node tools/build.mjs \
					--compiler-root "$root" \
					--lane jir \
					--optimizer on \
					--lane-report \
					--scheduler boundary
		) >&2
	fi

	local sidecar="$port/target/js-build/$mode/boundary/program.js.lane.json"
	local artifact="$port/dist/$mode/boundary/fp.esm.min.js"
	local package_artifact="$port/dist/$package_mode/boundary/fp.esm.min.js"
	for path in "$sidecar" "$artifact" "$package_artifact"; do
		if [ ! -f "$path" ]; then
			echo "alloc gate: the build produced no $path" >&2
			return 1
		fi
	done

	local work
	work=$(mktemp -d)
	# shellcheck disable=SC2064
	trap "rm -rf -- '$work'" RETURN
	"$terser" --module -c -m -- "$rescript/lib/es6/src/Hashing.res.mjs" >"$work/hashing.min.mjs"

	node - "$sidecar" "$work/hashing.min.mjs" "$artifact" "$package_artifact" \
		"$rescript/dist/fp.esm.min.js" "$oracle" "$worker" <<'NODE'
const fs = require("node:fs");
const { spawnSync } = require("node:child_process");
const [, , sidecar, kernelMin, candidate, packageCandidate, baseline, oracle, worker] = process.argv;

// The kernel's items. D9's names: the `X64` methods and the byte-block hash. The
// producer spells an inherent impl `impl_0_<method>`, so both shapes are matched.
const GATE =
  /\$hashing\$(X64\$(add|multiply|rotate_left|xor|xor_byte|final_mix)|impl_0_(add|multiply|rotate_left|xor|xor_byte|final_mix)|x64hash128_bytes)\$/;
// The module as a whole, which is what the ReScript file to compare against is.
const MODULE = /\$hashing\$/;
const LOOP_SITE_LIMIT = 0;
const ITEM_BYTE_LIMIT = 7686;
const RUNTIME_RATIO_LIMIT = 1.05;
const GZIP_BYTE_LIMIT = 62291;

const rows = JSON.parse(fs.readFileSync(sidecar, "utf8")).items ?? [];
if (rows.length === 0) throw new Error(`${sidecar} lists no items`);

// 1. in-loop allocation over the final JavaScript of the kernel items.
const gateRows = rows.filter((row) => GATE.test(row.name));
if (gateRows.length === 0) throw new Error(`${sidecar} has no hashing kernel items`);
const missing = gateRows.filter((row) => !row.js_alloc);
const jsOffenders = [];
const opaque = [];
const jirOffenders = [];
const views = [];
for (const row of gateRows) {
  for (const site of row.js_alloc?.sites ?? []) {
    jsOffenders.push(
      `${row.name} ${site.site} ${site.kind} depth=${site.depth} ` +
        `value=${site.value ?? "unknown"} jir=${site.jir_site ?? "unknown"} ` +
        `rust_type=${site.rust_type} shape=${site.physical_shape} ` +
        `reason=${site.materialization_reason}`,
    );
  }
  if ((row.js_alloc?.opaque_in_loop ?? 0) > 0) {
    opaque.push(`${row.name} opaque_in_loop=${row.js_alloc.opaque_in_loop}`);
  }
  for (const site of row.alloc?.sites ?? []) {
    const line = `${row.name} ${site.site} ${site.kind} depth=${site.depth} ${site.origin}`;
    if (site.kind === "materialize" || site.kind === "copy") jirOffenders.push(line);
    else if (site.kind === "view") views.push(line);
  }
}
const loopPass =
  jsOffenders.length <= LOOP_SITE_LIMIT && opaque.length === 0 && missing.length === 0;
console.log(
  `alloc gate murmur in-loop: ${loopPass ? "PASS" : "FAIL"} ` +
    `(${jsOffenders.length} final JS allocation sites over ${gateRows.length} kernel items` +
    `${opaque.length ? `; ${opaque.length} item(s) contain opaque loop code` : ""}` +
    `${missing.length ? `; ${missing.length} carry no js_alloc block` : ""}; gate <= ${LOOP_SITE_LIMIT})`,
);
for (const offender of jsOffenders) console.log(`  ${offender}`);
for (const item of opaque) console.log(`  ${item}`);
for (const offender of jirOffenders) console.log(`  (JIR materialize/copy) ${offender}`);
for (const view of views) console.log(`  (JIR view) ${view}`);
for (const row of missing) console.log(`  ${row.name}: no js_alloc block (lane ${row.lane})`);

// 2. size of the kernel, against the ReScript module minified the same way.
const moduleRows = rows.filter((row) => MODULE.test(row.name));
const rustBytes = moduleRows.reduce((total, row) => total + row.bytes_min, 0);
const rescriptBytes = fs.statSync(kernelMin).size;
const sizeRatio = rustBytes / rescriptBytes;
// A row that fell back has no JIR lowering to have measured, so its `bytes_min`
// is 0 and the sum below is an understatement. Say so rather than quietly pass.
const fellBack = moduleRows.filter((row) => row.lane !== "jir").length;
const sizePass = rustBytes <= ITEM_BYTE_LIMIT && fellBack === 0;
console.log(
  `alloc gate murmur size: ${sizePass ? "PASS" : "FAIL"} ` +
    `(${rustBytes} B over ${moduleRows.length} hashing items, gate <= ${ITEM_BYTE_LIMIT}; ` +
    `${sizeRatio.toFixed(2)}x / ${rescriptBytes} B ReScript` +
    `${fellBack ? `; ${fellBack} of them fell back and are not measured` : ""})`,
);

// 3. speed of the kernel. Same worker, same session, three runs, best ratio.
const measure = (artifact) => {
  const result = spawnSync(process.execPath, [worker, artifact, "15", "1000"], {
    encoding: "utf8",
  });
  if (result.status !== 0) {
    process.stderr.write(result.stderr || result.stdout);
    throw new Error(`kernel worker failed on ${artifact}`);
  }
  return JSON.parse(result.stdout.trim().split("\n").at(-1));
};
const truth = measure(oracle);
// Three runs of each artifact, and the best p50 each one reached. Pairing the two
// artifacts run by run instead would let one bad round of the baseline flatter the
// candidate: the ReScript p50 has been seen to swing 70% between runs of the same
// process. Best-of-three per artifact is the number neither side can be unlucky in.
const best = { candidate: {}, baseline: {} };
for (let round = 0; round < 3; round += 1) {
  for (const [side, artifact] of [["candidate", candidate], ["baseline", baseline]]) {
    const run = measure(artifact);
    for (const input of ["ascii", "unicode"]) {
      if (run.cases[input].hash !== truth.cases[input].hash) {
        throw new Error(
          `${side} ${input} hash ${run.cases[input].hash} != oracle ${truth.cases[input].hash}`,
        );
      }
      best[side][input] = Math.min(best[side][input] ?? Infinity, run.cases[input].p50Ns);
    }
  }
}
const ascii = best.candidate.ascii / best.baseline.ascii;
const unicode = best.candidate.unicode / best.baseline.unicode;
const kernelRatio = Math.sqrt(ascii * unicode);
const kernelPass = kernelRatio <= RUNTIME_RATIO_LIMIT;
console.log(
  `alloc gate murmur kernel: ${kernelRatio.toFixed(2)}x ${kernelPass ? "PASS" : "FAIL"} ` +
    `(ascii ${ascii.toFixed(2)}x ${(best.candidate.ascii / 1000).toFixed(2)} vs ` +
    `${(best.baseline.ascii / 1000).toFixed(2)} us, ` +
    `unicode ${unicode.toFixed(2)}x ${(best.candidate.unicode / 1000).toFixed(2)} vs ` +
    `${(best.baseline.unicode / 1000).toFixed(2)} us, best of 3, gate ${RUNTIME_RATIO_LIMIT})`,
);

// 4. Whole-package compressed size. Use the same deterministic gzip invocation as the
// FingerprintJS measurement harness; the fallback is only for hosts without the executable.
const source = fs.readFileSync(packageCandidate);
const compressed = spawnSync("gzip", ["-9", "-n", "-c"], { input: source });
const gzipBytes = compressed.status === 0
  ? compressed.stdout.length
  : require("node:zlib").gzipSync(source, { level: 9, mtime: 0 }).length;
const gzipPass = gzipBytes <= GZIP_BYTE_LIMIT;
console.log(
  `alloc gate idiomatic gzip: ${gzipPass ? "PASS" : "FAIL"} ` +
    `(${gzipBytes} B, frozen post-slice gate <= ${GZIP_BYTE_LIMIT})`,
);

process.exit(loopPass && sizePass && kernelPass && gzipPass ? 0 : 1);
NODE
}

mode=fixtures
fixtures=()
while [ "$#" -gt 0 ]; do
	case "$1" in
	--fingerprintjs)
		mode=fingerprintjs
		shift
		;;
	-*)
		echo "alloc-gate-test.sh: unknown option: $1" >&2
		exit 2
		;;
	*)
		fixtures+=("${1%.rs}")
		shift
		;;
	esac
done

if [ "${SKIP_BUILD:-0}" != 1 ]; then
	"$script_dir/build.sh" >/dev/null
fi

if [ "$mode" = fingerprintjs ]; then
	run_fingerprintjs
else
	run_fixtures ${fixtures[@]+"${fixtures[@]}"}
fi
