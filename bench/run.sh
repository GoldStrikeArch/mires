#!/usr/bin/env bash
#
# One measurement pass of the real krausest harness, end to end.
#
#   bench/run.sh            # the real thing: 6 implementations, ~2 hours
#   bench/run.sh --quick    # a subset, minutes, for proving the pipeline
#
# Requires bench/setup.sh to have run (it clones and installs the harness and
# builds the four reference implementations). This script only ever handles OUR
# two entries, then drives the harness over all six and writes bench/RESULTS.md.
#
# WHAT IT NEEDS FROM THE ENTRY LANES
# ----------------------------------
# One directory, a complete krausest implementation (index.html plus its
# artifacts plus a package.json carrying a `js-framework-benchmark` block, which
# is what makes the server's /ls list it):
#
#   bench/vanilla/dist-krausest/    -> frameworks/non-keyed/mires-vanilla
#
# They are COPIED, not symlinked. A symlink would leave the benchmarked bytes
# changing under the harness while it runs, and would make "what exactly was
# measured" unanswerable afterwards; the copy is a snapshot, and `ls -l` on the
# framework directory dates it.
#
# WHY THE isKeyed GATE IS A GATE
# ------------------------------
# Both of our entries are submitted as non-keyed, and an entry filed in the
# wrong category is not a slow result, it is an invalid one -- the whole point
# of the keyed/non-keyed split is that the two are not comparable. The harness
# ships the detector upstream uses on submissions, so we run it as a hard gate
# before spending two hours: if it says an entry is miscategorised, or that its
# row HTML does not match the contract, nothing is benchmarked.
#
# WHY THE MACHINE HAS TO BE IDLE
# ------------------------------
# 03/04/05/09 run under a 4x CPU throttle and 06 under 2x, which means the
# benchmark deliberately makes the machine the bottleneck -- and anything else
# competing for it lands in the measurement multiplied by the same factor.
# Chrome also runs NOT headless on purpose (an occluded or backgrounded window
# gets its rendering throttled by the compositor), so the window has to stay
# visible and the pointer has to stay out of it. Hence the banner, and hence
# `caffeinate -dims`: a display sleep part-way through a two-hour run does not
# fail anything, it just quietly poisons the numbers after that point.

set -uo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)

bench="$script_dir"
clone="$bench/krausest"
pin_file="$bench/KRAUSEST_PIN"
raw_dir="$bench/results-raw"
log_dir="$root/build/logs"

# The entries, as: <framework dir name>|<source directory>|<label>
ENTRIES="
mires-vanilla|$bench/vanilla/dist-krausest|Entry B (standalone, hand-written DOM ops in Rust)
"

# The full six, in the order the results table reads best.
FRAMEWORKS="keyed/vanillajs keyed/solid keyed/leptos non-keyed/vanillajs non-keyed/mires-vanilla"

quick=0
for arg in "$@"; do
	case "$arg" in
		--quick) quick=1 ;;
		*) printf 'run.sh: unknown argument %s\n' "$arg" >&2; exit 2 ;;
	esac
done

# ------------------------------------------------------- caffeinate re-exec

# Re-exec under caffeinate rather than wrapping individual commands: the run is
# a long sequence and only the outermost process needs the assertion held.
if [ "${MIRES_BENCH_CAFFEINATED:-0}" != "1" ]; then
	export MIRES_BENCH_CAFFEINATED=1
	if command -v caffeinate >/dev/null 2>&1; then
		# -d display, -i idle, -m disk, -s system-on-AC. All four, because every
		# one of them is a way for the machine to go quiet mid-measurement.
		exec caffeinate -dims /usr/bin/env bash "${BASH_SOURCE[0]}" "$@"
	fi
	printf 'warning: caffeinate not found; the machine may sleep mid-run\n' >&2
fi

mkdir -p "$log_dir" "$raw_dir"

warnings=0

step() { printf '\n\033[1m=== %s ===\033[0m\n' "$*"; }
note() { printf '  %s\n' "$*"; }
warn() { warnings=$((warnings + 1)); printf '\033[33mwarning:\033[0m %s\n' "$*" >&2; }
fail() { printf '\n\033[31mrun.sh failed:\033[0m %s\n' "$*" >&2; exit 1; }

timed() {
	local label=$1; shift
	local start elapsed status
	start=$(date +%s)
	"$@"
	status=$?
	elapsed=$(($(date +%s) - start))
	printf '  [%s] %ds (exit %d)\n' "$label" "$elapsed" "$status"
	return $status
}

# ------------------------------------------------------------------ banner

cat <<'BANNER'

################################################################################
#                                                                              #
#   THE MACHINE HAS TO BE IDLE FOR THE NEXT WHILE.                             #
#                                                                              #
#   * Quit everything else. Five of the nine CPU benchmarks run under a 2x-4x  #
#     CPU throttle, so background load is measured MULTIPLIED.                 #
#   * Chrome will open a VISIBLE window. Do not minimise it, do not cover it,  #
#     do not switch spaces away from it -- an occluded window gets its frames  #
#     throttled and the paint numbers become fiction.                          #
#   * KEEP THE MOUSE POINTER OUT OF THE BROWSER WINDOW. A hover changes what   #
#     gets repainted.                                                          #
#   * Do not close the lid. caffeinate holds off sleep; it cannot help with a  #
#     lid.                                                                     #
#                                                                              #
################################################################################

BANNER

if [ "$quick" -eq 1 ]; then
	note "--quick: a subset run, for proving the pipeline. The numbers it produces are"
	note "         NOT the artifact -- too few iterations, and only some of the benchmarks."
fi
printf '\n'

# --------------------------------------------------------------- preflight

step "preflight"
for tool in node npm curl rsync; do
	command -v "$tool" >/dev/null 2>&1 || fail "$tool is not on PATH"
done
[ -d "$clone/.git" ] || fail "bench/krausest is not there -- run bench/setup.sh first"
[ -d "$clone/node_modules" ] || fail "bench/krausest is not installed -- run bench/setup.sh first"
[ -d "$clone/webdriver-ts/dist" ] || fail "webdriver-ts is not compiled -- run bench/setup.sh first"

pin=$(tr -d ' \t\n\r' < "$pin_file" 2>/dev/null)
head=$(git -C "$clone" rev-parse HEAD 2>/dev/null)
[ "$pin" = "$head" ] || fail "bench/krausest is at $head, not the pinned $pin. Re-run bench/setup.sh, which explains the options."
note "harness at pinned $pin"

if lsof -nP -iTCP:8080 -sTCP:LISTEN >/dev/null 2>&1; then
	fail "port 8080 is in use; the harness server needs it:
$(lsof -nP -iTCP:8080 -sTCP:LISTEN)"
fi
note "port 8080 is free"
note "node $(node -v), Chrome $("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" --version 2>/dev/null | awk '{print $3}')"

# ------------------------------------------------------- stage our entries

step "stage our entries"

missing_entries=""
staged_entries=""

# Clear both before staging either. Otherwise a copy left behind by an earlier
# run is still served, still discovered by /ls and still benchmarked -- a stale
# entry silently standing in for one that was not delivered this time, which is
# the one kind of wrong number nothing downstream can detect.
rm -rf "$clone/frameworks/non-keyed/mires-vanilla"

IFS=$'\n'
for spec in $ENTRIES; do
	[ -z "$spec" ] && continue
	name=${spec%%|*}
	rest=${spec#*|}
	src=${rest%%|*}
	label=${rest#*|}
	dest="$clone/frameworks/non-keyed/$name"

	if [ ! -d "$src" ]; then
		missing_entries="$missing_entries
    $name -- expected at $src
      ($label)"
		continue
	fi
	if [ ! -f "$src/package.json" ]; then
		fail "$src has no package.json. The harness discovers implementations by reading
frameworks/*/*/package.json for a \`js-framework-benchmark\` block; without one the server's /ls
will not list $name and it cannot be benchmarked."
	fi
	if [ ! -f "$src/index.html" ]; then
		fail "$src has no index.html -- the driver has nothing to navigate to"
	fi

	# Replace wholesale rather than sync into place: a leftover artifact from a
	# previous shape of the entry is served, counted in the size benchmark, and
	# invisible in a diff.
	rm -rf "$dest"
	mkdir -p "$dest"
	rsync -a --exclude node_modules --exclude .git "$src/" "$dest/" \
		|| fail "could not copy $src into $dest"
	note "$name <- $src ($(du -sh "$dest" | awk '{print $1}'))"
	staged_entries="$staged_entries $name"
done
unset IFS

if [ -n "$missing_entries" ]; then
	if [ "$quick" -eq 1 ]; then
		warn "not staged (this is a --quick run, so continuing with whatever is present):$missing_entries"
	else
		fail "our entries have not been delivered:$missing_entries

Each is produced by its own lane's build. A full run measures every implementation, so it
does not start without it. Use --quick to exercise the pipeline on the references alone."
	fi
fi

# The entries may carry their own dependencies (a bundler, say). Install and
# build only if they asked for it -- an entry whose artifacts are prebuilt and
# committed into its dist directory needs neither, and inventing an npm step for
# it would just be a slower way to copy files.
for name in $staged_entries; do
	dest="$clone/frameworks/non-keyed/$name"
	if [ -f "$dest/package-lock.json" ]; then
		timed "npm ci $name" npm --prefix "$dest" ci --no-audit --no-fund \
			|| fail "npm ci failed for our entry $name"
	elif node -e 'const p=require(process.argv[1]);process.exit(Object.keys({...p.dependencies,...p.devDependencies}).length?0:1)' "$dest/package.json"; then
		warn "$name declares dependencies but ships no package-lock.json; falling back to npm install (not reproducible)"
		timed "npm install $name" npm --prefix "$dest" install --no-audit --no-fund \
			|| fail "npm install failed for our entry $name"
	else
		note "$name has no dependencies; skipping install"
	fi

	if node -e 'const p=require(process.argv[1]);process.exit(p.scripts && p.scripts["build-prod"] ? 0 : 1)' "$dest/package.json"; then
		( cd "$dest" && timed "build-prod $name" npm run build-prod ) \
			|| fail "npm run build-prod failed for our entry $name"
	else
		note "$name has no build-prod script; serving the delivered files as they are"
	fi
done

# ------------------------------------------------------------ start server

step "harness server"

server_pid=""
cleanup() {
	if [ -n "$server_pid" ]; then
		# The whole process group: `npm start` is a shell that spawns node, and
		# killing only the shell leaves Fastify holding 8080 for the next run.
		kill -TERM -- "-$server_pid" 2>/dev/null
		sleep 1
		kill -KILL -- "-$server_pid" 2>/dev/null
		wait "$server_pid" 2>/dev/null
		server_pid=""
	fi
}
trap cleanup EXIT INT TERM

set -m
( cd "$clone" && exec npm start ) > "$log_dir/bench-run-server.log" 2>&1 &
server_pid=$!
set +m

ls_json=""
for _ in $(seq 1 60); do
	ls_json=$(curl -fsS --max-time 5 http://localhost:8080/ls 2>/dev/null)
	[ -n "$ls_json" ] && break
	sleep 1
done
[ -n "$ls_json" ] || fail "the harness server never answered GET :8080/ls -- see $log_dir/bench-run-server.log"
note "server up, /ls answering"

# Which of the six does the server actually see? Everything downstream is
# filtered to that, so a --quick run with no entries staged still works.
present=$(printf '%s' "$ls_json" | node -e '
	const text = require("node:fs").readFileSync(0, "utf8");
	const found = new Set(JSON.parse(text).map(f => `${f.type}/${f.directory}`));
	console.log(process.argv.slice(1).filter(name => found.has(name)).join(" "));
' $FRAMEWORKS) || fail "could not read the /ls payload"

for name in $FRAMEWORKS; do
	case " $present " in
		*" $name "*) ;;
		*) warn "the server does not list $name; it will be left out of this run" ;;
	esac
done
[ -n "$present" ] || fail "the server lists none of the six implementations"
note "benchmarking: $present"

# -------------------------------------------------------------- isKeyed gate

step "isKeyed gate"

gated=""
for name in $staged_entries; do
	case " $present " in
		*" non-keyed/$name "*) gated="$gated non-keyed/$name" ;;
	esac
done

if [ -z "$gated" ]; then
	note "none of our entries are staged; nothing to gate"
else
	# Run per entry rather than in one invocation, so a failure names the entry
	# in the shell's output as well as in the detector's.
	for target in $gated; do
		( cd "$clone/webdriver-ts" && timed "isKeyed $target" npm run isKeyed -- "$target" ) \
			|| fail "isKeyed rejected $target.
It reports three things -- keyedRun, keyedRemove, keyedSwap -- and an aggregate; the aggregate must
be false for an entry submitted under non-keyed/. It also checks the row HTML against the benchmark
contract, so a shape problem in the markup fails here too. Read its output above: benchmarking a
miscategorised entry would produce a number that cannot honestly be compared with anything."
	done
	note "all staged entries are correctly categorised as non-keyed"
fi

# ------------------------------------------------------------------ bench

step "benchmark"

if [ "$quick" -eq 1 ]; then
	# One CPU benchmark, one memory benchmark and the size/first-paint trio: the
	# three different result shapes, so report.mjs is exercised on all of them,
	# at 3 iterations instead of 15.
	quick_args="--benchmark 01_ 21_ 40_ --count 3"
	note "quick: $quick_args"
else
	quick_args=""
	note "full: 15 iterations, all benchmarks, ~13-15 min per implementation"
fi

bench_log="$log_dir/bench-run-$(date -u '+%Y%m%dT%H%M%SZ').log"
note "log -> $bench_log"

# The runner writes into webdriver-ts/results/ and never clears it, so a file
# from an earlier run survives one that did not re-measure that pair -- and
# results-raw/ would then be a mixture of two runs presented as one. Move the
# old ones aside instead of deleting them: a previous run's numbers are still
# evidence, they just are not THIS run's.
if [ -n "$(ls -1 "$clone/webdriver-ts/results/"*.json 2>/dev/null)" ]; then
	archive="$clone/webdriver-ts/results-$(date -u '+%Y%m%dT%H%M%SZ')"
	mkdir -p "$archive"
	mv "$clone/webdriver-ts/results/"*.json "$archive/"
	note "archived $(ls -1 "$archive" | wc -l | tr -d ' ') result files from an earlier run -> $(basename "$archive")/"
fi

# shellcheck disable=SC2086
( cd "$clone" && timed "bench" npm run bench -- --framework $present $quick_args ) 2>&1 | tee "$bench_log"
bench_status=${PIPESTATUS[0]}
if [ "$bench_status" -ne 0 ]; then
	warn "the benchmark runner exited $bench_status. Results for whatever completed are still
         collected below, but check $bench_log before quoting any of them."
fi

# The runner reports a per-implementation error summary rather than failing, so
# a run can "succeed" with a framework that errored every benchmark.
if grep -q "ERROR" "$bench_log"; then
	warn "the benchmark log contains ERROR lines; grep '$bench_log' for ERROR before trusting the table"
fi

# ---------------------------------------------------------------- results

step "collect results"

# The upstream results app is a nice-to-have, not our artifact: it also rebuilds
# a React bundle, which is a lot of machinery to let fail a measurement run.
( cd "$clone" && timed "npm run results" npm run results ) \
	|| warn "npm run results failed; RESULTS.md does not depend on it (it reads the per-benchmark JSON directly), but the upstream results app will be stale"

produced=$(ls -1 "$clone/webdriver-ts/results/"*.json 2>/dev/null | wc -l | tr -d ' ')
[ "$produced" != "0" ] || fail "the harness wrote no result files into webdriver-ts/results/ -- see $bench_log"

rm -f "$raw_dir"/*.json
cp "$clone/webdriver-ts/results/"*.json "$raw_dir/" || fail "could not copy the result files"
# createResultJS also writes one aggregate; kept alongside under a name that
# sorts first and cannot collide with a per-benchmark file.
if [ -f "$clone/webdriver-ts/results.json" ]; then
	cp "$clone/webdriver-ts/results.json" "$raw_dir/_aggregate.json"
fi
note "$produced result files -> bench/results-raw/"

cleanup
trap - EXIT INT TERM

# ----------------------------------------------------------------- report

step "report"
node "$bench/report.mjs" || fail "report.mjs failed"

step "done"
note "raw     bench/results-raw/ ($produced files)"
note "report  bench/RESULTS.md"
note "log     $bench_log"
if [ "$warnings" -gt 0 ]; then
	note "warnings $warnings (see above)"
fi
if [ "$quick" -eq 1 ]; then
	printf '\nThis was a --quick run. RESULTS.md is a draft, not the artifact.\n'
fi
