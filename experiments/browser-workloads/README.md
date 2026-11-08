# Browser workloads

Ten reduced Rust workloads, each shaped after an established browser library (MiniSearch, Marked,
fflate, idb, Ky, uPlot, Workbox, and a composed Explorer). They are compiled to JavaScript by the
mires v2 backend. They are diagnostic recreations, not full ports. [REPORT.md](REPORT.md) covers
their scope, the correctness repairs they drove, and the admission verdict.

```sh
npm --prefix experiments/browser-workloads run build                  # dist/<workload>/index.min.js, results/build.json
node experiments/browser-workloads/tools/run.mjs --pure                # Node checks + timing
node experiments/browser-workloads/tools/run.mjs \
  --browsers chrome,firefox,safari                                     # + browser checks, lifecycle, browser timing
```

`run.mjs` options: `--samples 9`, `--warmups 2`, `--min-sample-ms 20`, `--seed 1`,
`--output <path>` (default `results/runtime.json`), `--build-receipt <path>`, and `--cycles 8`
(service-worker lifecycle, Chrome only). `--pure` behaves as before: Node correctness checks,
`pureBenchmarks`, no browsers. It now also runs the Node timing cells described below.

An invalid option value is a usage error: the run exits 1 with a message, before any work and
without writing a receipt.

- `--samples` must be an integer of at least 1.
- `--warmups` must be an integer of at least 0. Zero is allowed, matching `measure.mjs`.
- `--seed` must be an integer.
- `--min-sample-ms` must be a positive number.
- `--browsers` names must be `chrome`, `firefox` or `safari`. An unknown name is a usage error,
  as in `measure.mjs`. `unavailable` is reserved for a known browser whose WebDriver session
  could not start.

## Runtime timing against JavaScript references

**These numbers are not a performance claim.** This harness and its receipts make runtime
comparisons possible. The receipts in this tree were recorded on a 10-core machine while other
agents were compiling, at a load average of about 5 to 25. Absolute times are noisy. Read every
ratio next to the A/A control and the `loadAverage` recorded beside it.

Three workloads are timed: marked, minisearch and fflate, the same set and inputs as
`pureBenchmarks`. In each environment, every timed workload runs two **cells**:

- **rust/js**: the Rust build against the original JavaScript library doing the same work on the
  same input.
- **A/A**: the Rust build against a second, separately imported instance of the same artifact. Its
  ratio should be about 1.0, and its distance from 1.0 shows how noisy that environment was during
  that run.

Environments: Node (always), plus each browser named in `--browsers` (`chrome`, `firefox`,
`safari`).

### JavaScript references

Each reference is pinned exactly in the root `package.json` devDependencies and in
`package-lock.json`. The ports do not name the library versions they were derived from, so the
pins are the newest releases that were at least two weeks old on 2026-11-08. An adapter in
[`reference/`](reference/) exposes the same `run(options)` signature as the Rust entry. It imports
the package's published ESM entry by a relative path into `node_modules`. `run.mjs` checks that
Node's package resolution reaches that same file, and that the adapter imports it.

| workload | reference | adapter | compared (canonical form) |
| --- | --- | --- | --- |
| marked | `marked@18.1.1`, `lib/marked.esm.js` | [`reference/marked.mjs`](reference/marked.mjs) | the full HTML string, byte for byte |
| minisearch | `minisearch@7.2.0`, `dist/es/index.js` | [`reference/minisearch.mjs`](reference/minisearch.mjs) | the set of matched document ids |
| fflate | `fflate@0.8.3`, `esm/browser.js` (the browser build, also in Node) | [`reference/fflate.mjs`](reference/fflate.mjs) | every gzip byte except header offsets 8 and 9 |

How each pair was checked for equivalence (each check reproduces on every run through the gate
below):

- **marked.** The Rust port renders a line-based Markdown subset. The adapter changes one thing in
  marked: its `html` renderer escapes `&`, `<` and `>`, because the port has no raw HTML and
  escapes it. On the timed input (`makeMarkdown(80)`, 12,860 bytes of output) and on the
  `# Hi …` correctness input, the two outputs are identical. marked also escapes `"` and `'` in
  text and the port does not. The timed input contains neither character.
- **minisearch.** Both index the same `id|title|body` rows: title weight 3, body 1, terms longer
  than one character, lower-cased. Both answer `browser comp*` as an OR of `browser` and the prefix
  `comp`. MiniSearch scores with BM25+, and the port sums integer weighted term frequencies, so
  scores and tie order differ by design. Only the matched id set is compared: all 12 documents
  (also checked at 2 and 240 documents).
- **fflate.** `gzipSync(strToU8(text), {level: 0, mtime: 0})` emits stored DEFLATE blocks, as the
  port's `gzip_store` does. On the 61,264-byte timed input, the 61,287-byte outputs differ only at
  offset 8 (XFL: fflate 4, port 0) and offset 9 (OS: fflate 3, port 255). The checksum zeroes those
  two bytes. fflate computes CRC-32 with a lookup table and the port computes it bit by bit, so
  this ratio measures algorithm choices as well as compiler output. For an input whose length is a
  positive multiple of 65,535, fflate appends an empty final block, and such an input would fail the
  gate.

The exact difference lists are in `TIMED_WORKLOADS` in
[`tools/timing-protocol.mjs`](tools/timing-protocol.mjs) and are copied into every receipt under
`timing.workloads`.

### Protocol

All protocol code is in [`tools/timing-protocol.mjs`](tools/timing-protocol.mjs). The file is
browser-safe, so Node and every browser run the same code. A cell runs in isolation: in Node, a
fresh process ([`tools/timing-cell.mjs`](tools/timing-cell.mjs)); in a browser, a fresh page load.
Inside the cell:

1. **Cold import.** Each arm is loaded with `import()`, in seeded random order, and each import
   is timed separately (`coldImportMs`). The A/A arms import the same artifact under two URLs.
   Browsers fetch every module from a loopback server with `cache-control: no-store` and a unique
   query per page load. Browser-internal compilation caches are not controlled.
2. **Gate.** Each arm makes one call (`firstCallMs`, not a sample). The canonical SHA-256
   checksums of the two arms must agree, and must equal the Node rust/js checksum for that workload.
   The input's canonical-JSON SHA-256 must equal the one `run.mjs` computed. If any check fails, the
   cell records `checksum-mismatch` or `input-mismatch` and stops; it never reports timings.
3. **Calibration.** Each arm runs until `--min-sample-ms` has elapsed. Its `iterationsPerSample`
   is the larger of the `pureBenchmarks` call count (marked 5, minisearch 4, fflate 3) and the
   number of calls that fills `--min-sample-ms`. JavaScript arms therefore run many more calls per
   sample than Rust arms. All comparisons are per call.
4. **Warmups and samples.** `--warmups` rounds, then `--samples` rounds. In each round, the two arms
   run back to back. The order is seeded random in balanced blocks of two rounds (AB+BA or BA+AB),
   so each order runs equally often, within one. The cell yields to the event loop between rounds,
   outside the timed region. Timing uses `performance.now()`.
5. **Determinism.** After the samples, each arm is called once more, and its checksum must still
   match. Otherwise the cell status is `nondeterministic`.

Reported per arm: per-call `p50`/`p95`/`min`/`max` in milliseconds (sample time divided by
`iterationsPerSample`), `sampleMs` (per-sample statistics), cold import, first call and checksums.
Reported per cell: `ratio.p50` = first arm per-call p50 / second arm per-call p50 (rust/js, or
rust-a/rust-b for A/A). Also reported are `ratio.p95`, `ratio.min`, and `ratio.paired`, the
distribution of per-round ratios between two adjacent runs. That last one is the most robust
estimate while the load drifts.

### Browsers

`--browsers` runs the existing browser checks first, plus the service-worker lifecycle in Chrome.
Each browser then gets a separate WebDriver session (selenium-webdriver 4.41) for timing, with one
fresh page load per cell. Chrome runs with `--headless=new` and Firefox with `-headless`. Safari has
no headless mode. When `chromedriver` or `geckodriver` is not on `PATH`, Selenium Manager downloads
it into `~/.cache/selenium`. The driver version is recorded. The timing page is served with
`Cross-Origin-Opener-Policy: same-origin` and `Cross-Origin-Embedder-Policy: require-corp`.
`crossOriginIsolated` and the measured timer resolution are recorded per cell; observed values are
5 µs in Chrome and 20 µs in Firefox.

The adapters and their library files are served at their repository-relative URLs. That way, the
adapters' relative imports resolve as they do in Node. Firefox does not apply a page's import map
to modules imported from a WebDriver script, so the page uses no import map.

Safari needs remote automation turned on once, with `safaridriver --enable` (requires an
administrator password). If a browser's WebDriver session cannot start, its timing environment is
recorded as `unavailable` with the driver's error, never as a number, and `status.timingPassed` is
false.

### Receipt fields (`results/runtime.json` → `timing`)

| field | content |
| --- | --- |
| `protocol` | protocol version and a description of each step |
| `loadAverage` | `os.loadavg()` (1/5/15 min) at the start and end of the run; every cell also carries its own start/end |
| `host` | platform, architecture, CPU model, logical CPUs |
| `references` | per workload: package, pinned and installed version, entry file path/bytes/SHA-256, adapter path/bytes/SHA-256, `nodeResolvesSameEntry`, `adapterImportsEntry` |
| `workloads` | per workload: input description and canonical-JSON SHA-256, base iterations, what is compared, known differences |
| `environments` | per environment (`node`, `chrome`, …): `status` (`ok`, `failed`, `unavailable`), `reason`, identity (Node and V8 version, or browser name, version, platform, driver version, user agent, headless), and the raw cell results |
| `rows` | one row per arm: environment, workload, pair, implementation (`rust` or `js`), arm, status, `inputSha256`, `iterationsPerSample`, `warmups`, `samples`, `p50`/`p95`/`min`/`max` (ms per call), `sampleMs`, `coldImportMs`, `firstCallMs`, `checksum`, `rawChecksum`, `checksumMatchesNode`, timer resolution, `crossOriginIsolated`, `loadAverage` |
| `comparisons` | one row per cell (`rust/js` or `A/A`): status, `ratio` (p50/p50), `ratioP95`, `ratioMin`, `pairedRatio`, checksum, `loadAverage`. An unavailable environment gets one row with its status and reason. |

`run.mjs` also prints a summary table: per environment and workload, Rust and JS p50/p95 in ms
per call, rust/js, paired, A/A, and whether the checksums were equal.
[`experiments/v2-restoration/measure.mjs`](../v2-restoration/measure.mjs) `--browsers` passes the
browser list through and records these fields in its `runtime.json` and RESULTS.md.

### Limits

- The machine load is not controlled. The A/A ratio and `loadAverage` show it but do not remove
  it.
- A rust/js ratio compares two implementations of the shared behaviour, not two compilations of
  the same algorithm. That applies to the fflate CRC and to MiniSearch's radix-tree index against
  the port's linear scans.
- The Rust arm's per-call time includes the port's host-boundary work, which the JS reference
  does not pay. Rust/JS therefore overstates the cost of compiler output by that fraction.
  - fflate: the entry (`entries/fflate.rs`) builds its `Uint8Array` with one `Reflect.set` per
    output byte (`host::set_index`), 61,287 calls per call. The harness verifier measured about
    2.6 ms of a 6–14 ms call.
  - marked and minisearch: each call does a `Reflect.get` per input field and copies the
    string into the Rust heap. The result is converted back from the heap to a JS string
    (`JsValue::erase`).

  The protocol does not separate this work from the rest of the call.
- The A/A control is a noise control, not two independent runtimes. The second Rust instance
  shares `globalThis.__rt` with the first. Each import reassigns the runtime's maps and helpers,
  so both instances run on the runtime state of whichever was imported last. Each instance has
  its own compiled program functions and JIT feedback.
- Cold imports are single observations per page load or process. They include the loopback fetch,
  parse, compile and module evaluation.
- Iteration counts come from one calibration per arm, so two runs can use different counts. All
  ratios are per call.
