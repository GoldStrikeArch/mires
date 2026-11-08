# v2 restoration measurement loop

`measure.mjs` builds the declared target matrix with fixed flags, measures what the builds
produced, and records it. The package build tools only compile (rustc + the mires backend), bundle
(Rollup) and minify (Terser); every number in a receipt is measured by `measure.mjs` itself.

```sh
node experiments/v2-restoration/measure.mjs --label R0
  [--targets compact,idiomatic,pro,corpus]   # default: all four
  [--no-build]                               # measure existing artifacts only (built: false)
  [--pro-timeout-min 15]                     # wall-clock limit for the Pro build
  [--pro-optimizer on|off]                   # off = the optimizer-off control build
  [--samples 9 --warmups 2]                  # corpus Node timing
  [--force]                                  # replace an existing build/v2-restoration/<label>/
node --test experiments/v2-restoration/measure.test.mjs   # timeout / process-tree kill / parsers
```

Each run writes `build/v2-restoration/<label>/` and appends one block to [RESULTS.md](RESULTS.md).
Pro observations are kept out of both; see [Private Pro observations](#private-pro-observations).
An existing label directory is never overwritten unless `--force` is passed. RESULTS.md is
append-only: a re-run under the same label appends a second block. The script exits 0 when it
recorded the run, including runs where a build failed or timed out; those outcomes are data, shown
as statuses. It exits 1, before measuring anything, when a required tool is missing, an option is
invalid, or the label directory exists.

## Target matrix (fixed flags)

Builds run one after another, never concurrently, and Node timings run only after every build.

| target | command (from the repository root) | artifacts read from |
| --- | --- | --- |
| compact | `node experiments/fingerprintjs-rust-compact/tools/build.mjs --profile release-balanced --optimizer on --lane jir --cleanup on --scheduler boundary --lane-report --output experiments/fingerprintjs-rust-compact/dist/mires-v2` | that output directory |
| idiomatic | `node experiments/fingerprintjs-rust/tools/build.mjs` with the same flags and `--output experiments/fingerprintjs-rust/dist/mires-v2` | `…/dist/mires-v2/boundary/` (the tool writes one subdirectory per scheduler) |
| pro | `node experiments-pro/fp-pro-rust/tools/build.mjs --profile release-balanced --optimizer on --lane jir --cleanup on --lane-report --output experiments-pro/fp-pro-rust/dist/mires-v2` (`--optimizer off` and `…/dist/mires-v2-opt-off` with `--pro-optimizer off`) | that output directory (main and worker) |
| corpus | `node experiments/browser-workloads/tools/build.mjs`, the same command as `npm run build:corpus` without npm in the timed command | `experiments/browser-workloads/dist/<workload>/` |

`--lane-report` makes the tools publish the link lane reports (`link-lane-report.json`,
`lane-report.json`) that the decision counts come from. The idiomatic and Pro tools keep their own
defaults for flags not listed above. In particular, they leave `--rust-opt-level` unset and
`--mir-inline` off, while the compact tool defaults to `z` and `on`. Pro is private and local only. Its
build tool must exist when `pro` is selected. The Pro tool writes the minified worker to both
`fp.worker.js` and `fp.worker.min.js`, so the worker's `raw` column equals its `min` column.

Every build runs with `RCGJS_COST_PROFILE=1` in its environment. Any `rcgjs-cost-profile:` lines
that reach the build log are parsed into `compile-cost.json`; their absence is not an error. Each
link prints one block of `key=value` lines. The parser splits blocks where a key repeats, or where an
`output=` key appears. A linker that names its output must print that key first in its block. Each block is
recorded as `{order, output, attributedBy, counters}`. `attributedBy` is `"output"` when the block
names its link and `"log-order"` when only its position in the log identifies the link. The current
jsc-link names no output, so its blocks are `log-order`.

## Private Pro observations

Pro is private, so its observations never enter public files.

- **Pro receipts** go to `build/v2-restoration/<label>/pro/`: `inputs.json` with the Pro command,
  `sizes.json`, `decisions.json`, `compile-cost.json`, `summary.json` and `logs/`. The rest of the
  label directory can be archived or published without that subdirectory.
- **Pro blocks** go to `experiments-pro/v2-restoration/RESULTS.md`. That file is git-ignored by
  `/experiments-pro/` and append-only, and uses the same block format: label, date, HEAD, the
  artifact table and the link table.
- **The public RESULTS.md block** gets one Pro line: `pro: <status>`, where the status is `ok`,
  `timed-out`, `failed`, or `skipped` (not built). When the build completed, the line adds its wall
  time and peak RSS. It has no byte counts and no file names. The public `summary.json` carries the
  same fields under `privateTargets`, and the other public receipts leave Pro out.

## Receipts

Files are relative to `build/v2-restoration/<label>/`. For Pro they are relative to
`build/v2-restoration/<label>/pro/`, which has no `runtime.json` or `receipts/`.

| file | content |
| --- | --- |
| `inputs.json` | Git HEAD and `git status --porcelain` count and lines, compiler source digest, sha256 of `target/release/jsc-link` and `librustc_codegen_mires.dylib`, node/rustc/rollup/terser/zlib/brotli versions, host, options, exact commands, UTC timestamp |
| `sizes.json` | per artifact: bytes, sha256, gzip, brotli, mtime, status, and the unminified counterpart's bytes; fetched totals; corpus cross-check against `results/build.json` |
| `compile-cost.json` | per build: status, exit code, wall/user/sys seconds, max RSS, command, log path, the `cargo` crates compiled during the build, whether the sysroot was rebuilt, cost-profile blocks (`costProfile.links[]` with `order`, `output`, `attributedBy`, `counters`), and the kill report after a timeout |
| `decisions.json` | per link (Pro: main and worker; corpus: per workload): applied and would-apply counts (sroa, ast_dce, inline, devirt), range, inline declines by reason (including `inline.exceptional-control-flow`), fallback count, ABI/physical/repr decisions, item lanes, optimizer-report totals, and emitted-code counts from `raw/program.js`: bytes, `^function ` items, `try{`, `if(__rt.is_abort(`, `__rt.` helper references by name. Each source report's path, sha256 and status are included. |
| `runtime.json` | corpus `run.mjs --pure` p50/p95/min/max per timed workload (marked, minisearch, fflate) and the offline kernel checks (`experiments/tools/check-oss-kernels.mjs`) for compact and idiomatic |
| `summary.json` | the headline numbers from all of the above |
| `logs/` | `<target>-build.log` (stdout+stderr), `<target>-build.time.txt` (`/usr/bin/time -l`), runtime and kernel logs |
| `receipts/` | the sub-tools' own receipts: corpus build and runtime receipts, kernel receipts |

### Artifact status

| status | meaning |
| --- | --- |
| `ok` | written during this run's successful build |
| `existing` | `--no-build`: whatever is on disk, not produced by this run |
| `stale` | a build ran, but the file predates it (left over from an earlier build) |
| `unverified` | written during this run, but the build failed or timed out |
| `missing` | not on disk |

Build statuses are `ok`, `failed` (non-zero exit), `timed-out` and `not-built`.

## Protocol

- **Compression.** Only Node zlib: `gzipSync(buf, {level: 9})` and `brotliCompressSync` with quality
  11 and lgwin 22. The Node, zlib and brotli versions are recorded. The package checkers
  (`tools/check-package.mjs`) and the corpus build receipt compute `gzip9` with the system
  `gzip -9 -n` when it is present. Those gzip numbers are a separate dataset and are not compared.
  The corpus cross-check therefore compares bytes, sha256 and brotli and only reports gzip.
- **Fetched resources.** Each fetched resource is counted once. A package's fetched total is its ESM
  entry bundle, plus the worker bundle for Pro. Shims are already inside the bundles, and the iife/umd
  bundles are alternatives to the ESM one, so neither is added. `raw/program.js` rows are linker
  output, not shipped files.
- **Link cost.** Each package build runs under `/usr/bin/time -l`. Wall seconds cover the whole build
  tool: the `cargo` freshness check, the sysroot cache check, every rustc/jsc-link invocation,
  Rollup, Terser and the ES2018 gate. If `compile-cost.json` lists `cargoCompiled` crates or reports
  `sysrootRebuilt: true`, that wall time is not a clean link-cost observation. On macOS, "maximum
  resident set size" is the largest single process among the build and its reaped descendants
  (rusage merges children by maximum), so it is not the sum of concurrent processes. It is reported in
  MiB (2^20 bytes).
- **Timeouts.** The Pro build runs in its own process group with a wall-clock limit
  (`--pro-timeout-min`, default 15). On expiry, the script sends SIGTERM to the process group and to
  every process in the build's process tree, snapshotted with `ps`. The snapshot catches a
  descendant that left the group. Anything still alive after 5 s gets SIGKILL. The build is recorded
  as `status: "timed-out"` with `timeoutMinutes` and a kill report (`survivors` must be empty), and
  never with a duration. Ctrl-C on `measure.mjs` stops the active build the same way.
- **Runtime.** `experiments/browser-workloads/tools/run.mjs --pure --samples N --warmups M` in Node.
  One sample is the time for `callsPerSample` calls, in milliseconds. p50 and p95 are order
  statistics over the samples. The run also checks the corpus outputs and that `dist/` matches
  `results/build.json`.
- **Compiler identity.** `sourceIdentity` reuses the method of
  [docs/v2-source-identity.json](../../docs/v2-source-identity.json). Its input is every git-visible
  (tracked or untracked, non-ignored) file under the recorded top-level directories plus the
  recorded root files. The digest is the sha256 of the compact, sorted-key JSON of
  `[{bytes, path, sha256}]` sorted by path. `methodVerified` confirms that this method reproduces the
  recorded digest from the recorded inventory. `matchesRecorded` and `changesFromRecorded` compare the
  current tree with that inventory.
