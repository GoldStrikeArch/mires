#!/usr/bin/env bash
#
# One-time (and re-runnable) preparation of the real krausest harness.
#
#   bench/setup.sh            # prepare; skip anything already done
#   bench/setup.sh --force    # redo the npm installs and the reference builds
#
# This clones github.com/krausest/js-framework-benchmark into bench/krausest,
# installs it, and builds the four reference implementations our entries are
# measured against. It does NOT build or run anything of ours -- that is
# bench/run.sh. Run this once; run.sh many times.
#
# WHY THE CLONE IS PINNED
# -----------------------
# The number in RESULTS.md is only meaningful next to the harness that produced
# it: krausest changes the benchmark definitions, the Chrome flags and the
# reference implementations themselves over time, and a re-run six months from
# now against a moved upstream would silently be measuring something else. So
# the FIRST run records the sha it happened to get into bench/KRAUSEST_PIN, that
# file is committed, and every later run either fetches exactly that sha or --
# if a clone is already sitting there at a different commit -- refuses and says
# so rather than quietly benchmarking the wrong tree. Deleting bench/krausest
# and re-running reproduces the pinned tree from scratch.
#
# WHY THE REFERENCE BUILDS ARE PART OF SETUP
# ------------------------------------------
# vanillajs (keyed and non-keyed), solid and leptos are the comparison, and all
# four are slow-ish npm installs that have nothing to do with our code. Doing
# them here means a run.sh invocation is only ever building OUR two entries, so
# the expensive-and-boring part is not repeated on every measurement attempt.
#
# leptos is the odd one: it is a Rust framework, and if its build-prod actually
# shelled out to cargo/trunk we would need a wasm toolchain we do not have. It
# does not -- the entry commits its built bundle and build-prod is a no-op copy
# -- but that is an upstream fact that could change, so it is asserted rather
# than assumed, and the fallback is to use the committed bundle and warn.

set -uo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)

bench="$script_dir"
clone="$bench/krausest"
pin_file="$bench/KRAUSEST_PIN"
stamps="$clone/.mires-setup"
log_dir="$root/build/logs"

upstream="https://github.com/krausest/js-framework-benchmark.git"

# The four reference implementations, as harness-relative directories.
REFERENCES="
keyed/vanillajs
non-keyed/vanillajs
keyed/solid
keyed/leptos
"

force=0
for arg in "$@"; do
	case "$arg" in
		--force) force=1 ;;
		*) printf 'setup.sh: unknown argument %s\n' "$arg" >&2; exit 2 ;;
	esac
done

mkdir -p "$log_dir"

warnings=0

step() { printf '\n\033[1m=== %s ===\033[0m\n' "$*"; }
note() { printf '  %s\n' "$*"; }
warn() { warnings=$((warnings + 1)); printf '\033[33mwarning:\033[0m %s\n' "$*" >&2; }
fail() { printf '\n\033[31msetup.sh failed:\033[0m %s\n' "$*" >&2; exit 1; }

# Wall-clock around a phase, because "how long does a cold setup take" is a
# question the next person will have and nobody ever writes it down.
timed() {
	local label=$1; shift
	local start elapsed
	start=$(date +%s)
	"$@"
	local status=$?
	elapsed=$(($(date +%s) - start))
	printf '  [%s] %ds (exit %d)\n' "$label" "$elapsed" "$status"
	return $status
}

# ---------------------------------------------------------------- preflight

step "preflight"
for tool in git node npm curl; do
	command -v "$tool" >/dev/null 2>&1 || fail "$tool is not on PATH"
done
note "node $(node -v), npm $(npm -v), git $(git --version | awk '{print $3}')"

if lsof -nP -iTCP:8080 -sTCP:LISTEN >/dev/null 2>&1; then
	fail "port 8080 is already in use -- the harness server needs it (the /ls smoke at the end of this script, and every run.sh). Free it and re-run:
$(lsof -nP -iTCP:8080 -sTCP:LISTEN)"
fi
note "port 8080 is free"

# A phase is done when its stamp exists. The stamp directory lives INSIDE the
# clone (so it is gitignored, and so deleting the clone resets everything), which
# means it must not be created before the clone step below has run -- an early
# mkdir would leave a bench/krausest that is not a git checkout and the pin check
# would refuse it.
done_already() {
	[ "$force" -eq 0 ] && [ -f "$stamps/$1" ]
}
mark_done() {
	mkdir -p "$stamps"
	date -u '+%Y-%m-%dT%H:%M:%SZ' > "$stamps/$1"
}

# ------------------------------------------------------------ clone and pin

step "clone at pin"

if [ -d "$clone/.git" ]; then
	head=$(git -C "$clone" rev-parse HEAD 2>/dev/null)
	if [ ! -f "$pin_file" ]; then
		printf '%s\n' "$head" > "$pin_file"
		note "no pin file; recorded the existing clone's HEAD $head"
	fi
	pin=$(tr -d ' \t\n\r' < "$pin_file")
	if [ "$head" != "$pin" ]; then
		fail "bench/krausest is at $head but bench/KRAUSEST_PIN says $pin.
A benchmark run against a different upstream tree is not comparable with the recorded one.
Either \`git -C bench/krausest checkout $pin\`, or delete bench/krausest and re-run to
re-fetch the pin, or -- if you MEAN to move the pin -- update KRAUSEST_PIN deliberately."
	fi
	note "clone present at pinned $pin"
else
	[ -e "$clone" ] && fail "bench/krausest exists but is not a git clone; remove it and re-run"
	if [ -f "$pin_file" ]; then
		pin=$(tr -d ' \t\n\r' < "$pin_file")
		note "fetching pinned $pin (shallow)"
		mkdir -p "$clone"
		git -C "$clone" init -q                                  || fail "git init failed"
		git -C "$clone" remote add origin "$upstream"            || fail "git remote add failed"
		timed "fetch" git -C "$clone" fetch -q --depth 1 origin "$pin" \
			|| fail "could not fetch $pin from $upstream (a sha older than the server's reachability window cannot be fetched shallowly; delete KRAUSEST_PIN to re-pin at HEAD, and say so in RESULTS.md)"
		git -C "$clone" checkout -q FETCH_HEAD                   || fail "git checkout FETCH_HEAD failed"
	else
		note "no pin recorded; cloning HEAD and pinning to it"
		timed "clone" git clone -q --depth 1 "$upstream" "$clone" || fail "git clone failed"
		pin=$(git -C "$clone" rev-parse HEAD)
		printf '%s\n' "$pin" > "$pin_file"
		note "pinned at $pin"
	fi
fi

note "pin: $(tr -d ' \t\n\r' < "$pin_file")"
note "clone size: $(du -sh "$clone" 2>/dev/null | awk '{print $1}')"

# ---------------------------------------------------------- root npm install

step "npm ci (harness root)"
if done_already root-install; then
	note "skipped (stamped); --force to redo"
else
	if ! timed "npm ci" npm --prefix "$clone" ci --no-audit --no-fund; then
		# Upstream's root devDependencies do not satisfy each other's peer ranges
		# under npm 11 (at the pinned sha: eslint 10 vs eslint-plugin-react's
		# peer of <=9). Every package involved is lint/format tooling that the
		# benchmark never runs, and --legacy-peer-deps on a `ci` installs the
		# committed lockfile tree unchanged -- it relaxes the peer CHECK, not the
		# resolution -- so the harness we measure with is still exactly upstream's.
		# Retried rather than defaulted-to, so that the day upstream fixes this we
		# go back to a clean install without anyone editing this script.
		warn "npm ci failed at the harness root (upstream root devDependency peer conflict); retrying with --legacy-peer-deps"
		timed "npm ci (legacy peers)" npm --prefix "$clone" ci --no-audit --no-fund --legacy-peer-deps \
			|| fail "npm ci failed at the harness root even with --legacy-peer-deps -- see the output above"
	fi
	mark_done root-install
fi

step "npm run install-local (server, webdriver-ts, results app)"
if done_already install-local; then
	note "skipped (stamped); --force to redo"
else
	# install-local is the harness's own aggregate installer; it is a few hundred
	# megabytes and the single longest part of a cold setup.
	( cd "$clone" && timed "install-local" npm run install-local ) \
		|| fail "npm run install-local failed -- see the output above"
	mark_done install-local
fi

# -------------------------------------------------------- reference impls

# Does this package's build-prod reach for a Rust toolchain we do not have?
needs_rust_toolchain() {
	local pkg=$1
	node -e '
		const fs = require("fs");
		const pkg = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
		const script = (pkg.scripts && pkg.scripts["build-prod"]) || "";
		process.exit(/\b(cargo|trunk|rustup|wasm-pack|wasm-bindgen)\b/.test(script) ? 0 : 1);
	' "$pkg"
}

for ref in $REFERENCES; do
	dir="$clone/frameworks/$ref"
	stamp="ref-$(printf '%s' "$ref" | tr '/' '-')"

	step "reference $ref"
	[ -d "$dir" ] || fail "frameworks/$ref does not exist in the pinned tree -- upstream moved or renamed it"

	if done_already "$stamp"; then
		note "skipped (stamped); --force to redo"
		continue
	fi

	if [ "$ref" = "keyed/leptos" ] && needs_rust_toolchain "$dir/package.json"; then
		# Asserted, not assumed: see the header. If upstream ever makes this a
		# real Rust build we do not silently try (and fail) to run it.
		if [ -d "$dir/bundled-dist" ] && [ -n "$(ls -A "$dir/bundled-dist" 2>/dev/null)" ]; then
			warn "keyed/leptos build-prod now invokes a Rust toolchain: $(node -p 'JSON.parse(require("fs").readFileSync(process.argv[1],"utf8")).scripts["build-prod"]' "$dir/package.json")
         Using the committed bundled-dist/ instead and skipping the build. The leptos numbers in
         RESULTS.md are therefore from upstream's committed bundle, which is what the published
         table measures too."
			mark_done "$stamp"
			continue
		fi
		fail "keyed/leptos build-prod needs a Rust/wasm toolchain and there is no committed bundled-dist/ to fall back on"
	fi

	timed "npm ci" npm --prefix "$dir" ci --no-audit --no-fund \
		|| fail "npm ci failed for frameworks/$ref"
	( cd "$dir" && timed "build-prod" npm run build-prod ) \
		|| fail "npm run build-prod failed for frameworks/$ref"

	# What every entry must end up with is an index.html the driver can navigate
	# to -- NOT a dist/. Both vanillajs entries have a build-prod of `exit 0` and
	# serve their sources straight from the framework root, which is a perfectly
	# valid entry shape, so a dist/ check would warn about the two references
	# most likely to be fine.
	if [ ! -f "$dir/index.html" ] && [ ! -f "$dir/dist/index.html" ] && [ ! -f "$dir/bundled-dist/index.html" ]; then
		warn "frameworks/$ref built without an index.html in the framework root, dist/ or bundled-dist/ -- the driver may have nothing to navigate to"
	fi
	mark_done "$stamp"
done

# ------------------------------------------------------------- /ls smoke

step "server /ls discovery smoke"

server_pid=""
cleanup() {
	if [ -n "$server_pid" ]; then
		# Kill the whole process group: `npm start` is a shell that spawns node,
		# and killing only npm leaves Fastify holding port 8080 for the next run.
		kill -TERM -- "-$server_pid" 2>/dev/null
		sleep 1
		kill -KILL -- "-$server_pid" 2>/dev/null
		wait "$server_pid" 2>/dev/null
	fi
}
trap cleanup EXIT INT TERM

# Monitor mode so the background job gets its own process group to kill.
set -m
( cd "$clone" && exec npm start ) > "$log_dir/bench-setup-server.log" 2>&1 &
server_pid=$!
set +m

ls_json=""
for _ in $(seq 1 60); do
	ls_json=$(curl -fsS --max-time 5 http://localhost:8080/ls 2>/dev/null)
	[ -n "$ls_json" ] && break
	sleep 1
done
[ -n "$ls_json" ] || fail "the harness server never answered GET :8080/ls -- see $log_dir/bench-setup-server.log"

printf '%s' "$ls_json" > "$log_dir/bench-setup-ls.json"
note "/ls answered ($(printf '%s' "$ls_json" | wc -c | tr -d ' ') bytes) -> $log_dir/bench-setup-ls.json"

# /ls answers an array of framework descriptors read off each frameworks/*/*/
# package.json: {type: "keyed" | "non-keyed", directory: "<name>", ...}. That is
# the SAME discovery the benchmark driver does, which is the point of smoking it
# -- a directory the server cannot see is a directory that cannot be benchmarked,
# whatever is on disk. (It is discovery only, not a build check: at the pinned
# sha /ls lists every framework whose package.json parses, built or not.)
missing=$(LS_FILE="$log_dir/bench-setup-ls.json" node -e '
	const fs = require("fs");
	const wanted = process.argv.slice(1);
	const text = fs.readFileSync(process.env.LS_FILE, "utf8");
	let found;
	try {
		found = new Set(JSON.parse(text).map(f => `${f.type}/${f.directory}`));
	} catch {
		// Shape changed upstream; fall back to a substring scan rather than
		// failing the whole setup over a payload format.
		process.stderr.write("warning: /ls is not the expected array of descriptors; matching on substrings\n");
		found = { has: name => text.includes(name) };
	}
	console.log(wanted.filter(name => !found.has(name)).join(" "));
' $REFERENCES) || fail "could not read the /ls payload"

if [ -n "$missing" ]; then
	fail "the server's /ls does not list: $missing
The benchmark driver discovers implementations by scanning frameworks/, so an implementation it
cannot see cannot be benchmarked. Check that each missing directory built a dist/."
fi
note "all four reference implementations are discoverable"

cleanup
server_pid=""
trap - EXIT INT TERM

# ------------------------------------------------------------------ done

step "setup complete"
note "pin        $(tr -d ' \t\n\r' < "$pin_file")"
note "clone      $clone ($(du -sh "$clone" 2>/dev/null | awk '{print $1}'))"
note "references keyed/vanillajs non-keyed/vanillajs keyed/solid keyed/leptos"
if [ "$warnings" -gt 0 ]; then
	note "warnings   $warnings (see above)"
fi
printf '\nNext: bench/run.sh (see its header for what it needs from the entry lanes).\n'
