# Library × compiler matrix

The matrix compares defined API subsets of 15 JavaScript libraries across nine compiled languages. It checks exact output against each pinned npm release and measures production bundle size and time against that reference. A sixteenth workload, Explorer, links each language's own IndexedDB, search, Markdown, compression and chart ports into an offline application. The `pilot` program exercises the harness and `floor` measures each language's runtime overhead.

| Language id | Family | Production variant built here |
| --- | --- | --- |
| `rust` | Rust through mires (this repository) | `js-profile=release-size js-opt=on js-ecma-target=2018`, unwind mode, `JSC_RUST_OPT_LEVEL=z`, `js-lane=jir`; minimal shim, Rollup, Terser (ecma 2018) |
| `rescript` | ReScript 12.3.0 | ES module emit; Rollup + node-resolve over `@rescript/runtime`; common Terser |
| `melange` | Melange 7.0.1 (OCaml 5.3.0) | `melange.emit` es6 with the emitted stdlib; Rollup + node-resolve; common Terser |
| `jsoo` | js_of_ocaml 6.4.1 | whole program, `--opt=3 --effects=disabled --target-env=browser --no-extern-fs`; module facade; common Terser |
| `scalajs` | Scala 3.8.4, Scala.js 1.22.0 | `fullLinkJS` (`scala-cli --js-mode release`), ES modules; common Terser |
| `dart` | dart2js | `dart compile js -O2 --no-source-maps`; comparator facade; common Terser |
| `kotlin` | Kotlin/JS, Gradle plugin 2.4.10 | `compileProductionExecutableKotlinJs`, whole program, ES2015 ES module; Terser, 2 passes |
| `elm` | Elm 0.19.2 plus a JS host | `elm make --optimize`; comparator wrapper; compiler-aware Terser on the Elm module; Rollup over the host; common Terser |
| `gopherjs` | GopherJS 1.21.0 + Go 1.21.13 | `gopherjs build -m`; comparator facade; common Terser |

These are the production rows of [`docs/COMPARATORS.md`](../../docs/COMPARATORS.md) ("Which output
to compare"), built with the same toolchain invocations as the comparator ports
(`experiments/fingerprintjs-<family>/tools/build.mjs`). "Common Terser" is the comparator ports'
pass: `{ format: { comments: false }, safari10: true, module: true }`.

[`MATRIX.md`](MATRIX.md) reports parity, size and timings for each port. Start a new library
with its Rust port and check it against the pinned npm package before completing its other eight
ports. Record each mires compiler defect and its workaround in [`MIRES-BUGS.md`](MIRES-BUGS.md).
Runnable examples live in [`mires-bugs/`](mires-bugs/).

The browser workloads use real host APIs. In the Elm rows, JavaScript drives IndexedDB events, Fetch, the service worker and the Canvas context. Elm drives the transaction sequence, Ky retry decisions and chart geometry. The source and bundle figures in the report include the JavaScript host. Each workload's `CONTRACT.md` describes its API coverage and additional verifiers. [IndexedDB](libraries/idb/CONTRACT.md) · [Ky](libraries/ky/CONTRACT.md) · [uPlot](libraries/uplot/CONTRACT.md) · [Workbox](libraries/workbox/CONTRACT.md) · [Explorer](libraries/explorer/CONTRACT.md)

## Commands

Run from this directory (`npm --prefix experiments/library-matrix run <script>` works from the root).

```sh
node tools/build.mjs --library pilot --language all      # 9 builds, one receipt each
node tools/check.mjs --library pilot                     # exact parity: node, chrome, firefox, safari
node tools/bench.mjs --library pilot                     # timing: node, chrome, firefox (safari)
node tools/sizes.mjs                                     # static sizes, LOC, runtime floors
node tools/matrix.mjs                                    # MATRIX.md + matrix-results.json

npm run pilot                                            # all five for the pilot (and the floor)
npm run all                                              # all five for every library
node tools/selftest.mjs                                  # the harness's own logic, no toolchains (Chrome and Firefox for the realm test)
```

| Tool | Options |
| --- | --- |
| `build.mjs` | `--library <ids\|all>` `--language <ids\|all>` (both required), `--clean` (delete the port's work directory first, so no Dune `_build`, Gradle project, `.scala-build`, `elm-stuff` or staged copy carries over; global caches such as `~/.gradle`, `~/.elm` and `GOCACHE` stay), `--timeout 1200` (seconds per port) |
| `check.mjs` | `--library` (required), `--language`, `--environments node,chrome,firefox,safari` (default: the library's list), `--readable` (also check `dist/index.js`), `--call-timeout 30` (seconds a call may run without progress before the cell is `hang`) |
| `bench.mjs` | `--library` (required), `--language`, `--environments`, `--design triplet\|pairs`, `--samples 24`, `--warmups 2`, `--min-sample-ms 20`, `--seed 1`, `--call-timeout 30`, `--aa-range 0.5,2` (an A/A p50 outside it refuses the cell) |
| `sizes.mjs` (partial run) | `--library` / `--language` update those rows of the shared `sizes.json` and keep every other library's rows |
| `selftest.mjs` | `--receipts <dir>` (its scratch modules go to `<dir>/selftest`; the directory must be inside the repository, whose files the browser test serves) |
| `sizes.mjs` | `--library`, `--language` |
| `matrix.mjs` | `--output <dir>` (default: this directory) |
| all | `--matrix <registry>` (default `matrix.json`), `--receipts <dir>` (default `build/library-matrix`; the build work directories go to `<dir>/work`) |

An invalid option is a usage error (exit 2) before any work. `build.mjs`, `check.mjs` and
`bench.mjs` exit 1 when any cell failed; `sizes.mjs` and `matrix.mjs` exit 1 and write nothing when
any port's `dist/` is stale.

## Layout

```text
matrix.json                 registry: environments, languages, libraries, ports
libraries/<id>/             one library: CONTRACT.md, reference.mjs (adapter), corpus.json
                            (stand-ins also hold package/, the module standing in for npm)
languages/common.mjs        shared builder helpers: staging, Rollup, Terser, shape gate, dist/ writer
languages/opam.mjs          the local OCaml switch (Melange and js_of_ocaml)
languages/<language>/       build.mjs, plus shared templates (kotlin/gradle-template, gopherjs/go.sum)
ports/<library>/<language>/ one port: port.json and sources; dist/ is build output (git-ignored)
tools/                      build, check, bench, sizes, matrix, selftest, and the per-cell entry points
tools/lib/                  registry and receipts (Node), canonical forms, check runner and timing
                            protocol (browser-safe), browser driving and its page-realm bootstrap
MATRIX.md, matrix-results.json   generated report

build/library-matrix/       (repository build/, git-ignored)
  builds/<library>--<port>.json            build receipts
  logs/, attempts/<key>/<ns>/              build logs; the previous receipt and log of every rebuild
  previous-dist/<library>/<port>/          the dist/ moved aside by the last build
  checks/<library>--<port>--<env>.json     parity receipts (one per cell), <library>.summary.json
  bench/<library>/<key>--<env>--<case>--<design>.json   timing receipts (one per cell), <library>.summary.json
  sizes.json                               size receipt
  work/<library>/<port>/                   toolchain work directories (caches, staged sources)
```

## Port convention

Every port builds to **`dist/index.min.js`**, the production artifact, and **`dist/index.js`**, the
same compile before the final minifier pass (readable where the toolchain allows; for some families
the compiler output itself is already compact). Both are **ES modules with no imports** (static or
dynamic) and a **named export `run(input)`**. The builder also writes `dist/meta.json`
(toolchain, settings, attribution; written without timestamps, so a rebuild can be
byte-identical; see "Reproducibility").

- `input` is one corpus case's JSON-compatible value.
- The result is a value of the library's canonical form, or a Promise of one when the library is
  declared async: a string, a `Uint8Array`, or JSON (see "Checker").
- **No cross-call memoisation.** A port must not cache, memoise or precompute anything across
  calls that the reference does not (a module-level map keyed by the input, a counter, a lazily
  built table the reference rebuilds per call). Black-box checks cannot see a correct content cache,
  so this is a rule, backed by the bench-variant hook ("Timing") and the `module state` lint
  ("Sizes"), whose every flagged line needs a reason in the port's review.
- **Self-contained at run time.** A port loads no module, reads no file, touches no network and
  creates no global beyond its family's documented runtime globals: the runtime guard ("Checker")
  fails a port that does, unless the library declares `runtimeGuard.network: "allowed"`
  (host-API libraries such as Ky). The same holds for reference adapters: an adapter reads no file
  at run time (its data belongs in the corpus or in a module it imports).
- **Fail fast.** A port reports an internal failure by throwing. Rust ports' `#[panic_handler]`
  throws (`mires_js::throw_value`, as in `ports/pilot/rust/src/lib.rs`), never `loop {}`, which
  hangs the caller; a hang is stopped by the cell watchdog and recorded as `hang`.
- **The boundary is part of the port.** Each port converts the JS input to its own types and the
  result back, idiomatically, and pays that cost inside `run`, timed with everything else. The
  harness does no conversion. Each `port.json` says, in `boundary`, what its port does. The pilot's
  boundaries:

| Language | Input | Output |
| --- | --- | --- |
| Rust | `JsValue`; `Reflect.get` binding; `read::<&str>()` (host string as Rust UTF-8), `read::<usize>()` | `JsValue::erase(string.as_str())` |
| ReScript | a record type, which is the JS object itself (no conversion) | the JS string |
| Melange | `< text : string; top : int > Js.t` read with `##` (no conversion) | the JS string |
| js_of_ocaml | class type; `Js.to_string` (UTF-16 to an OCaml UTF-8 string) | `Js.string` |
| Scala.js | `@js.native` facade trait (no conversion) | the JS string |
| Dart | `dart:js_interop` extension type (`String`, `int`) | `.toJS` |
| Kotlin/JS | `external interface` (no conversion) | the JS string |
| Elm | the JS host sends the input through a port; `Json.Decode` | a port message the host returns |
| GopherJS | `*js.Object`; `.Get(...).String()` (UTF-16 to Go UTF-8), `.Int()` | Go string to JS string |

Allowed techniques are the comparator contract's ("Allowed techniques" in
[`experiments/fingerprintjs-compare/CONTRACT.md`](../fingerprintjs-compare/CONTRACT.md)):
idiomatic code in the target language; no product JavaScript; Elm is the exception, with a JS host
under `js/` and the "mostly JS" label when the host is over half of the bundle; compiler emit
untouched, plus loading-only packaging. The packaging rules per family:

- **Native ES module output, used as emitted:** ReScript, Melange (Rollup bundles the emitted
  stdlib modules), Scala.js (`@JSExportTopLevel("run")`), Kotlin/JS (`@JsExport`), Rust (a facade
  re-exports the program's `#[js_export] fn matrix_run` as `run`; a three-letter export could
  collide with a minified local, which mires warns about).
- **Classic scripts, wrapped by a generated facade:** js_of_ocaml (`Js.export "run"`; the facade binds
  `module` so `jsoo_exports` is a fresh object, nothing is written to `globalThis`), GopherJS
  (`js.Module.Get("exports").Set("run", ...)`; the comparator facade binds `window` and `module`; the
  emit ships untouched with GopherJS's documented import side effects), Dart (`main()` publishes
  `run` on `globalThis.__matrixExports`; the comparator facade binds `self` and undefines
  `window`/`document`/`navigator`/`screen`/`location`, reads the object once and deletes the global).
- **Elm:** the port's JS host (`js/index.js`) imports the compiled program as `elm-program` and
  exports `run`. The comparator wrapper makes one change to the emit, inherited from
  `experiments/fingerprintjs-elm/tools/wrap-elm.mjs`: the outgoing-port manager's
  `_Process_sleep(0)` becomes `_Scheduler_succeed(null)`, so a response reaches the host
  synchronously on every call (Elm otherwise sleeps one timer tick after each dispatch).

Ports never re-implement the build: a port is sources plus `port.json`. Builders generate the
project files (`rescript.json`, `dune-project` and `dune`, `project.scala`, `pubspec.yaml`, the Gradle
project from `languages/kotlin/gradle-template`, `elm.json`, `go.mod`) in the work directory, next to
a fresh copy of the port's sources, so a deleted source can never survive in a toolchain's input
tree and no toolchain writes into the port.

### `port.json`

| Field | Languages | Meaning |
| --- | --- | --- |
| `schema`, `library`, `language` | all | `1`, the library id, the language id (must match the builder) |
| `boundary` | all | one sentence: how input and result cross the boundary |
| `entry` | rust, rescript, melange, jsoo, dart | entry source (defaults `src/lib.rs`, `src/Index.res`, `src/index.ml`, `src/main.ml`, `src/main.dart`) |
| `crateName` | rust | crate name (default `matrix_<library>`) |
| `main`, `host`, `elmJson` | elm | `src/Main.elm`, `js/index.js`, and an `elm.json` replacing the default (elm/core 1.0.5, elm/json 1.1.3) |
| `package`, `require`, `goSum` | gopherjs | package path (default `./src`), extra module requirements, extra `go.sum` lines file |
| `dependencies` | scalajs, dart, kotlin | `using dep` coordinates, a pub dependency map, Gradle coordinates |
| `libraries`, `rescript` | melange/jsoo, rescript | extra Dune libraries; extra `rescript.json` fields |

## Registry: `matrix.json`

- **`environments`**: `node`, and `chrome`, `firefox`, `safari` with their WebDriver name.
- **`languages.<id>`**: `label`, `builder`, `production` (the variant, in words), `sources`
  (extensions), `hostJs` (directories allowed to hold handwritten JS: Elm's `js`), `runtimeGlobals`
  (the globals the family's runtime documents creating: GopherJS `fs`; mires `__rt` and the
  `rustc_codegen_js.abort-errors.v1` registry symbol, written `@@<description>`). The comment
  syntax, inline-JS markers, eval idioms and module-state patterns per language live in
  `tools/lib/source-stats.mjs`.
- **`libraries.<id>`**: `label`; `npm` (`package`, exact `version`, `entry` inside the package,
  optional `modules: "harness"` when installed under this directory) or, for a stand-in, `package`
  with `standIn`; `reference` (the adapter); `contract`; `corpus`; `output` (`form`: `string`,
  `bytes` or `json`; `async`); `normalisation` (`null`, or `{ name, description, module }`);
  `environments` (where parity must hold); `bench` (`cases`, `baseIterations`, optional `variant`,
  or `null`); `runtimeGuard` (optional; `{ "network": "allowed" }` lifts the network guard); `role:
  "floor"` for the floor library.
- **`ports.<library>`**: language ids (the port lives in `ports/<library>/<language>/`), or
  `{ id, language, dir }` for a port elsewhere (used by the negative controls).

## Adding a library

1. **Pin the package.** The three libraries already in the root `package.json` (minisearch 7.2.0,
   fflate 0.8.3, marked 18.1.1) resolve from the root `node_modules`. Do not add root dependencies:
   add any other package to this directory's `package.json` `devDependencies` with an exact version,
   install it here (`npm install --prefix experiments/library-matrix`), and set `npm.modules:
   "harness"`.
2. **Write the adapter** `libraries/<id>/reference.mjs`: `export function run(input)`, importing the
   package's published ESM entry **by relative path** (no bare specifiers, no import map; the
   browsers load it from the same repository-relative URLs). It maps the corpus input onto the
   package API and adds no behaviour. The tools check that the installed version equals the pin
   and that the adapter imports the entry.
3. **Write `libraries/<id>/CONTRACT.md`** from this template:

   ```markdown
   # Library contract: `<id>`
   | Field | Value |
   | --- | --- |
   | Package | `<name>@<version>`, entry `<path>` |
   | Reference adapter | `reference.mjs`: what `run(input)` calls |
   | API subset | the exported functions and options the ports implement, and what they leave out |
   | Input | the JSON shape of a corpus input |
   | Output form | `string` / `bytes` / `json` (key order significant), sync or async |
   | Normalisation | none, or the precise named rule and why it is unavoidable |
   | Parity environments | node, or the browsers a host-API library needs |
   | Corpus | `corpus.json`, N cases |
   | Bench cases | case ids, baseIterations, and the bench variant (or why there is none) |
   | Caching | what the package itself caches across calls (ports may cache only that) |
   | Files | none: neither the adapter nor a port reads files at run time |

   ## Behaviour       the subset, precisely enough to port without reading the package source
   ## Parity domain    inputs the contract excludes, and why (e.g. exact float ties)
   ## Corpus           one line per case: what it exercises
   ```
4. **Write `corpus.json`**: `{ "schema": 1, "library": "<id>", "cases": [{ "id", "note", "input" }] }`.
   Include empty inputs, non-ASCII and astral text, boundary values, and the inputs the bench uses.
5. **Declare a bench variant** where the library allows one: `libraries/<id>/bench-variant.mjs`
   exporting `variant(input, k)` (k >= 1), a distinct input per k that (a) changes content
   throughout, leaving no large sub-input (prefix, suffix, document, field) unchanged across k, so
   no cache on a part of the input can hit, (b) keeps the work per call equivalent, (c) is
   deterministic and inside the parity domain; register it as `bench.variant`. The pilot's rotates
   the words by k and replaces a k-chosen word with a nonce. Every timed call then gets its own variant, and every verified output is
   checked against the reference on the same variant (see `libraries/pilot/bench-variant.mjs`).
   Without one, a port that caches by input content would time a cache hit.
6. **Register it** in `matrix.json` (`libraries` and `ports`), then build, check, bench, size.

## Adding a port

1. Create `ports/<library>/<language>/port.json` and the sources (layout per language as in the
   pilot: `src/` for all; Elm also `js/index.js`).
2. Add the language id to `ports.<library>` in `matrix.json`.
3. `node tools/build.mjs --library <id> --language <language>`, then `check.mjs`, then the rest.
4. Exact parity in every environment the library names is the bar. A port that builds but fails
   parity stays in the matrix with its failure.

## Builders and toolchain setup

Each `languages/<language>/build.mjs` compiles one port with the family's production settings and
packages the ESM; its header comment states every setting and where it comes from. Toolchains are
the comparator ones, set up by the existing scripts (`docs/COMPARATORS.md`, "Install and
reproduce"); the builders never install or upgrade one.

| Language | Needs | Set up by |
| --- | --- | --- |
| rust | the compiler built at this checkout (`target/release`), the keyed sysroot for the release-size arguments, rustup's pinned nightly | `bash scripts/build.sh --locked` (the builder runs it; a no-op when current) with `CARGO_INCREMENTAL` unset; the builder builds a missing keyed sysroot through `scripts/build_sysroot.sh` (`SKIP_BUILD=1`) |
| rescript, elm | root `node_modules` (`rescript` 12.3.0, `elm` 0.19.2; Elm packages in `~/.elm`) | `npm ci` at the root; `ELM_BIN` overrides Elm |
| melange, jsoo | opam root `.toolchains/opam`, switch `mires-comparators`; `DUNE_JOBS=4` | `npm run setup:comparators:ocaml` (`MIRES_OPAM_ROOT`, `MIRES_OPAM_SWITCH` override) |
| scalajs | `scala-cli` on PATH (the Coursier bin directory is put first), Java; Maven Central (`COURSIER_REPOSITORIES`) | `brew install scala-cli` |
| dart | `dart` on PATH, SDK `^3.13.0` | an installed Dart SDK; `DART` overrides |
| kotlin | Gradle 9.4.1 at `experiments/fingerprintjs-kotlin/build/toolchains/gradle-9.4.1` (downloaded and SHA-256-verified by that comparator's build), Java; Maven Central | `node experiments/fingerprintjs-kotlin/tools/build.mjs` once; `GRADLE_BIN` overrides |
| gopherjs | GopherJS 1.21.0 and Go 1.21.13 under `.toolchains/go`, `GOCACHE` there | `npm run setup:comparators:go` (`MIRES_GO_ROOT` overrides) |
| browsers | Chrome, Firefox; drivers via Selenium Manager (`~/.cache/selenium`) | selenium-webdriver 4.41 in the root `node_modules`; Safari needs `safaridriver --enable` (admin) |

The Kotlin Toolchain lane (`KOTLIN_BIN`) is the comparator's readable/proxy variant and is not
built here: the production row is the Gradle lane. The Kotlin builder instantiates one shared Gradle
project (`languages/kotlin/gradle-template`) per port under `build/library-matrix/work/`, with
`kotlin.srcDir` pointing at the staged sources; no port carries a Gradle project.

## Build receipts

`tools/build.mjs` follows `experiments/tools/run-comparator-builds.py`: it hashes the port's build
inputs (the port minus `dist/`, its builder directory, `languages/common.mjs`,
`languages/opam.mjs`, the root `package.json` and `package-lock.json`) before and after the build,
moves the old `dist/` to `previous-dist/` so it can never pass for fresh output, runs the builder
in its own process group with a timeout (`exitCode` 124 and `timedOut: true` when it expires),
records every artifact's bytes and SHA-256, re-runs the module shape gate (no imports, `run`
exported), and keeps the previous receipt and log under `attempts/`. `built` requires a zero exit,
`dist/index.min.js`, a passing shape gate, unchanged inputs and `meta.json`. When the previous
receipt had the same inputs, `reproducibility.identical` says whether every artifact is
byte-identical. The Rust builder's `build-info.json` (in the receipt as `buildInfo`) adds the
compiler revision, the compiler-tree digest before and after, and the binary hashes.

**Reproducibility.** `identical` compares one rebuild with the previous receipt; it is evidence,
not a guarantee, and MATRIX.md's "reproduced" means exactly that. Kotlin/JS needed a setting: with
the comparator's settings, clean Kotlin builds differed run to run (the supertype order in
`initMetadataForClass(...)` comes from identity-hashed sets; 3 distinct pilot outputs in 4 clean
builds, 2 in 3 for the FingerprintJS sources). The shared Gradle template runs the compiler in
process with a constant identity hash (`-XX:+UnlockExperimentalVMOptions -XX:hashCode=2`, a JVM
setting, no compiler flag or emit change), after which 8/8 clean pilot builds and 4/4 FingerprintJS
builds were byte-identical. The Rust `meta.json` hashes the artifact input graph, which holds
absolute paths: Rust builds reproduce in place, not across checkout locations. After any rebuild
that changes an artifact, its check and timing receipts are stale and must be re-run.

**Freshness.** Every consumer (checker, timing, sizes, report) first proves that the port's `dist/`
is the output of its current receipt: the receipt says built, every recorded artifact still has its
digest, `dist/` holds nothing unrecorded, and the build inputs still match `sourceAfter`. Otherwise
the port is `stale` (or `not-built` / `build-failed` when there is no successful receipt): the
checker and the timing tool refuse to run it and exit 1, and for a stale port `sizes.mjs` and
`matrix.mjs` refuse to write anything. The per-cell receipts are authoritative;
`<library>.summary.json` describes the last invocation only.

**Bindings.** Every check and timing receipt records what it was measured under, and is current
only while all of it still holds; otherwise it is `stale` everywhere: the timing tool will not gate
on it, and the report shows `stale` and counts nothing from it.

- **The library binding** (`tools/lib/registry.mjs` `libraryBinding`): the corpus digest; the
  reference identity, which is the adapter and every module it reaches through static imports,
  re-exports and literal dynamic `import()`s (a computed `import()` is an error, since it cannot be
  bound), the npm version and entry digest, and the digest of the whole package directory (the
  installed npm package, or the stand-in's directory, following symbolic links to their content and
  recording each link), so a file in the package read at run time is bound too; the
  normalisation module and its module graph; the output form; the bench-variant module and its
  graph.
- **The harness code**, by content: `harness.sha256` over `tools/lib/canonical.mjs`,
  `check-runner.mjs`, `tools/check.mjs`, `tools/check-cell.mjs`, `browser.mjs`, `page.mjs`,
  `cells.mjs` and `registry.mjs` for check receipts, and over `canonical.mjs`, `protocol.mjs`,
  `tools/bench.mjs`, `tools/bench-cell.mjs`, `browser.mjs`, `page.mjs`, `cells.mjs` and
  `registry.mjs` for bench receipts (`HARNESS_CODE` in `registry.mjs`).
  The protocol version strings are kept for people; any edit to these files makes their receipts
  stale. Build receipts record `harness` for `tools/build.mjs` and the Node version.
- **The environment's major version.** A Node receipt is current only under the same Node major as
  the tool reading it. The timing tool gates only on a check receipt from the same major version of
  the browser it is timing in, and the report shows a browser's timing as `stale` when it was
  measured in another major version than that environment's parity. Minor and patch updates do not
  invalidate receipts; every receipt records the full version.

The sizes receipt binds the JS reference row the same way. A partial `sizes.mjs` run updates only
its rows and libraries; every row carries `measuredAt`, and the report checks each row against the
current artifact, so a library is never dropped from the report by a partial run.

## Checker

One cell is one port in one environment: a fresh Node process (`tools/check-cell.mjs`) or a fresh
page load in a WebDriver session, running `tools/lib/check-runner.mjs`. The reference adapter is
imported and run on every case first, then the port, so nothing the port does at import time can
reach the reference.

**Page realm.** In a browser, the page itself loads the code. WebDriver inserts the same-origin
bootstrap `tools/lib/page.mjs` as a `<script type="module">` and waits for its result, and the
bootstrap imports the runner, which imports the reference and the port. So all of them run in the
page's realm (`globalThis === window`), as on any web page, never in the realm of the WebDriver
script. In Firefox that realm is Marionette's sandbox: a module imported from it has its own
`Error`, `Object` and `Uint8Array`, so host errors (`fetch`, `URL`, `Cache.put`) fail
`instanceof Error`, IndexedDB values and `structuredClone` results are objects of another realm, and
`TextEncoder` output is a cross-compartment `Uint8Array`, an order of magnitude slower to index.
The cell's request and its result cross WebDriver as JSON text (WebDriver argument objects lose
their key order); the result event carries a per-load token that only the bootstrap holds.
`tools/selftest.mjs` checks the realm in Chrome and Firefox, for check and bench cells.

Each case runs as a **call sequence on one input object**, identically on both sides: `first` (a
fresh `structuredClone` of the case input), `repeat` (the same object again), and `rewritten` (the
same object with its contents replaced in place by the next case's input, when both are plain
objects or both arrays). Every step compares the canonical output **and the input's canonical JSON
after the call**. So a port that mutates its argument differently from the reference, or that caches
by input identity (it returns a stale result on `rewritten`), fails the case; a case's
`failedStep` says where.

Outputs are compared by **canonical text, exactly** (`tools/lib/canonical.mjs`):

- `string`: the value must be a string; its canonical text is `JSON.stringify(value)`, exact for
  every JS string (lone surrogates are escaped).
- `bytes`: the value must be a `Uint8Array`; the text is its hex.
- `json`: a strict serializer: plain objects with their key order (order is part of the output),
  arrays, strings, booleans, `null`, `undefined`, numbers with `-0`, `NaN` and the infinities kept
  apart, BigInts. Any other object is an error, never a silent conversion.

A library may declare one **named normalisation** (`normalisation: { name, description, module }`,
a browser-safe module exporting `apply(value)`), applied to both sides before canonicalisation and
recorded with its module digest in every receipt. The default is none. When the reference throws,
the port must throw too (`both-threw`; messages are recorded, not compared). Returning a value the
output form rejects (a `Map` for `json`) is not throwing: it fails the case, and a reference that
does it makes the case `reference-error`. Returning a Promise for a synchronous library, or the
reverse, fails the case.

**Runtime guard.** The module shape gate at build time is syntactic (no `import` declaration, no
`import()` expression), so the checker also watches the code while it runs.

- **Port phase** (the port's import and every call), everywhere: `fetch`, `XMLHttpRequest`,
  `WebSocket`, `EventSource` and `navigator.sendBeacon` record a violation and throw, unless the
  library declares `runtimeGuard.network: "allowed"`; and the own property names (and symbol
  descriptions) of `globalThis` are compared before and after: a new global outside the family's
  `runtimeGlobals` is a violation (host-internal symbols such as Node's `undici.*` are ignored).
- **Node, port phase:** a synchronous module resolve hook (`module.registerHooks`,
  `tools/check-cell.mjs`) refuses every module load other than the port artifact: an `import()`
  built in an eval string, a `data:` URL, a Node builtin, and a module the process already loaded
  (the reference's own package). `process.getBuiltinModule`, `process.binding`,
  `process._linkedBinding` and `process.dlopen` throw. A guard error the code leaves unhandled (an
  unawaited `import()`) is recorded, not fatal, so it ends as a named violation.
- **Node, reference phase:** the same hook refuses Node builtins and the same process functions
  throw, so the reference adapter cannot read files at run time (it is an `error` of the library
  contract when it tries).
- **Browsers:** the check and bench pages carry a Content-Security-Policy that allows scripts,
  connections, images, frames and workers only from the loopback origin, so a `data:` or `blob:`
  module or script is refused and reported (`CSP refused …`); and the loopback server charges every
  request to the page load that made it (the `/__load/<N>/` token in its path, or in its Referer;
  each load is closed out before the next starts), which must ask only for the files the cell needs.
- A violation fails the cell (`runtime guard: …`), and a violation seen in Node refuses the port's
  timing in every environment (`runtime-guard`).

What the guard cannot see: in a browser, an `import()` of a module the page already loaded makes no
request (the Node cell catches it); a request whose page sets `referrerpolicy="no-referrer"` and an
absolute path cannot be attributed, and is charged to the cell running at the time; WebAssembly
compiled from inline bytes is not guarded (it is counted as an eval reference); `eval`, `Function`
and `import.meta` themselves are allowed (some runtimes need them; they are counted); reads of
globals the reference set up and changes to existing globals' values are not tracked.

**Threat model.** The harness catches accidental divergence and the common shortcuts: identity and
content caches (under bench variants), input mutation, delegation to the reference, network and
module loading, file reads, hangs, panics, and stale or rebound receipts. It is **not a security
sandbox** against a deliberately adversarial port: a port written to evade it can, for example
cache on a part of the input a variant leaves unchanged, or compute inside WebAssembly. Source
review of every port, together with the lint columns (`module state`, `eval refs`, `embedded JS`,
`host JS`), is part of every port's acceptance, not an optional extra.

**Hangs.** The cell sends a heartbeat before every call (Node: synchronous stderr lines; browsers:
`navigator.sendBeacon` to the loopback server). A cell silent for `--call-timeout` seconds
(default 30) is stopped (the Node process is killed; the browser session is quit and reopened for
the next cell) and recorded as `hang`, naming the last call it started, instead of blocking until a
whole-cell timeout.

Environments are Node first, then the browsers the library lists. Browsers are driven as in
`experiments/browser-workloads/tools/run.mjs` (selenium-webdriver, Chrome `--headless=new`, Firefox
`-headless`, files served at repository-relative URLs from an explicit whitelist with
`cache-control: no-store`, no import map). **Safari** needs "Allow Remote Automation", which is off
on this machine: its cells are `unavailable` with the driver's error, never a pass.
`referenceStableAcrossEnvironments` in the summary says whether the reference itself produced the
same digests everywhere.

## Timing

`tools/bench.mjs` runs the N-arm protocol of `tools/lib/protocol.mjs`, a generalisation of
`experiments/browser-workloads/tools/timing-protocol.mjs` (same steps, calibration rule, sink,
seeding and order statistics; that file is not modified). A cell runs in a **fresh Node process**
(`tools/bench-cell.mjs`) or a **fresh browser page load**, in the page realm as in the checker (the
input reaches the page inside the cell's JSON request, in its key order), and:

0. **parity gate** (in `bench.mjs`, before the cell starts): the port's check receipt for that
   environment must be a pass for the current artifact, library binding and check code, from the
   same major version of the environment; otherwise every cell of the port there is
   `parity-not-passed` and nothing is timed;
1. checks the input's canonical-JSON SHA-256 against the driver's;
2. cold-imports each arm in seeded random order, timing each import alone;
3. **gate**: one validation call per arm; every arm's canonical checksum must agree and equal the
   reference digest that the port's current check receipt recorded **in this environment**
   (`expectedChecksumSource` names it); otherwise the cell is `checksum-mismatch` and has no
   timings. Reference outputs may legitimately differ per engine: MiniSearch's BM25 scores go
   through `Math.log`, whose last bit is engine-specific. On 1,831 BM25 arguments, Node 26.8.1 and
   Chrome 158 disagree on 8.2%, Node and Firefox 160 on 2.2%, and Chrome and Firefox on 8.8%
   (Chrome 158 was correctly rounded on all of them). So parity and the gate are per environment,
   never Node's digest elsewhere;
4. calibrates each arm on cold code: `iterationsPerSample` = max(`baseIterations`, calls filling
   `--min-sample-ms`);
5. runs `--warmups` rounds, **re-calibrates every arm on warm code** (the samples use the warm
   value; both calibrations are recorded), then runs `--samples` rounds; each round runs every arm
   back to back, in orders from a **Williams design** (each block: the balanced Latin square with
   shuffled rows and permuted labels, mulberry32 seeded per cell), so every arm holds every
   position and directly follows every other arm equally often within a block (for two arms:
   AB/BA, the original protocol). The default 24 samples is four whole blocks for the triplet
   (6 orders) and twelve for pairs (2); `samplesBalanced` records whether a run's count is whole
   blocks;
6. calls every arm once more: its checksum must still match (`nondeterministic` otherwise).

**Inputs and outputs of timed calls.** Every call receives its own input, made outside the timed
region in batches bounded to 32 MB: a `structuredClone` of the case input, so no call sees an object
another call or arm has mutated or cached by identity, or, when the library declares a **bench
variant** (`bench.variant`), `variant(input, k)` from a k range disjoint per arm (k = 1 + armIndex
+ armCount × n), so no two timed calls share an input value, not even across the two instances of a
port that share the realm, and a content-keyed cache cannot hit either. After every
calibration, warmup and sample run, the outputs of two of its calls must have the expected
checksum (the gate checksum, or with variants the reference's output on the same variant, computed
untimed): one picked by a stream seeded from `crypto.getRandomValues` per cell (`pickSeed`, in the
receipt; a port cannot predict it from the cell's identity) and the last one. Otherwise the cell is
`output-mismatch` and has no ratio. A divergence on a fraction p of calls escapes with probability
about (1-p)^n for the n outputs checked per arm (the verifier measured n ≈ 46 per port in a
triplet), so low-rate divergence can escape but moves a ratio by about p at most.

**A/A bound.** Two instances of one artifact in one process or page cannot honestly differ by 2×: a
cell whose A/A p50 lies outside `--aa-range` (default [0.5, 2], recorded in every cell) is
`aa-out-of-range` and shows no ratio (the instances interfere, for instance through a cache shared
on `globalThis`, or the run was too noisy to read).

**Watchdog.** Cells send heartbeats (at most every 250 ms, never inside a timed batch); a cell silent
for `--call-timeout` seconds is stopped and recorded as `hang`, as in the checker.

Reported per arm: per-call p50/p95/min/max, per-sample statistics, cold import, first call,
checksums, iterations; per cell: `port/js`, `A/A` and `port-b/js` ratios of p50, p95, min, and the
paired ratio (median over rounds of the per-round ratio, the most robust while load drifts); per
cell the 1/5/15 minute load average at start and end. With fewer than 20 samples the p95 is the
maximum, and the receipt says so (`p95IsMax`).

**Cell design.** The default `triplet` cell has three arms: the port (instance `a`), the JS
reference, and a second, separately imported instance of the same artifact (`b`, a different URL,
so a different module instance). `port/js` is a/js and `A/A` is a/b, **from the same process or
page**. This is the sound alternative the brief allows to separate port-vs-js and A/A cells: the
noise control is measured under exactly the conditions of the ratio it qualifies (same load
moment, same JIT and GC state, same interleaving), instead of in another process a minute later, and
it costs one page load instead of two. The cost is that the port's second instance exists while the
ratio is measured (more code loaded, shared globals such as GopherJS's). `--design pairs` runs
browser-workloads' design instead (a port-vs-js cell and an A/A cell, each in its own process or
page) for comparison.

**These numbers are not a performance claim.** The machine load is not controlled: receipts in this
tree were recorded while other agents were compiling. The recorded pilot run (24 samples per cell, the
rotating bench variant on) saw 1-minute load averages of 4.5 to 6.0 on a 10-core machine and A/A
ratios of 0.90 to 1.02; an earlier run at a
load of 10 to 24 gave Firefox A/A ratios from 0.68 to 1.95 ([`tasks/lessons.md`](../../tasks/lessons.md)
records ±40% A/B swings on identical code above roughly 20), and the independent verification of
this harness saw A/A from 0.74 to 1.84 across seeds and designs at loads of 5 to 22. One run's A/A
range is not a property of the protocol. Read every ratio next to its A/A and load average. Limits, as in
`browser-workloads`:

- A port/js ratio compares two implementations of the shared behaviour, not two compilations of the
  same algorithm.
- The port's time includes its boundary conversions (by design: the boundary is part of the port).
- The A/A control is a noise control, not two independent runtimes: instances share the realm's
  globals (GopherJS's `fs`, the Dart facade's global, mires's shim registry symbol).
- Cold imports are single observations per process or page load; browser compilation caches are not
  controlled. Iteration counts come from one calibration per arm, so runs can differ; all ratios are
  per call.
- The pilot is tiny (a 3.5 KB input, 0.06–1.3 ms per call), so its ratios mostly measure boundary
  and runtime overheads.

## Sizes

`tools/sizes.mjs` measures `dist/index.min.js` (and `index.js`) like
`experiments/tools/measure-comparators.mjs`: raw bytes, gzip level 9 with `mtime: 0` and Brotli
quality 11, both from Node zlib, each resource on its own. It also counts:

The source statistics come from `tools/lib/source-stats.mjs`, a small per-language lexer that
knows each language's comments (nested where the language nests them) and string literals
(triple-quoted, template and OCaml quoted strings included); it is a review aid, not a parser
(OCaml and Rust character literals holding a quote, and Rust raw strings, can confuse it):

- **target-language lines**: lines with code outside comments in the language's source extensions;
- **embedded-JS lines**: lines spanned by inline-JS constructs (`js!{…}`, `%raw`/`%%raw`,
  `[%mel.raw …]`/`[%%mel.raw …]`, `Js.Unsafe.js_expr`/`eval_string`, `[%js …]`, Kotlin
  `js( "…")`/`js("""…""")`, `js.eval(…)`, `new js.Function(…)`), each counted once over its
  whole bracket group or string literal however its lines break; markers in comments or strings do
  not count;
- **eval references**: code loaded from bytes or built URLs (`WebAssembly.instantiate`/`compile`/
  `Module`/`Instance`, `data:` and `blob:` URLs in strings, `URL.createObjectURL`, matched on the raw
  text), and occurrences of eval or `Function` idioms (`#[js(call = "eval")]`, an
  `external` bound to `eval`, `Js.Unsafe.global##.eval`, `js.Dynamic.global.eval`,
  `js.Global.Get("eval")`, `callMethod('eval')`, and `eval(`, `new Function(`, `import(` and
  `import.meta` in handwritten JS);
- **module state**: module-level mutable declarations (Rust `static mut`/cell statics/
  `thread_local!`, top-level `ref`/`Map.make`/`Hashtbl.create` in ReScript and OCaml, object-level
  `var`/mutable collections in Scala, top-level `var`/`final` in Dart, top-level `var` or mutable
  collections in Kotlin, package-level `var` in Go, top-level `let`/`var`/`new Map` in host JS). A
  lint for the no-memoisation rule: every flagged line needs a reason (the pilot's two are the Elm
  host's `app` and pending-`response` slots);
- **host-JS lines**: handwritten `.js` in the allow-listed host directory (Elm's `js/`), and any
  handwritten JS elsewhere in a port, flagged as outside the allow-list. The **"mostly JS"** label
  uses the comparator's rule (raw host-JS bytes over the minified artifact above one half);
  `mostlyJs.renderedShare`, the host's share of the unminified bundle, is kept beside it;
- the **runtime floor**: the production artifact of the same language's `floor` port (the identity
  program, `libraries/floor`), measured identically for all nine families. Each builder's
  `meta.json` additionally keeps the family's own attribution of the emit (mires shim bytes,
  `@rescript/runtime` and Melange stdlib modules, js_of_ocaml's `--no-runtime` difference, Elm's
  empty-worker runtime and the host share, GopherJS's prelude and `$packages` blocks);
- the **JS reference, for scale**: the adapter and its package modules bundled by Rollup with the
  common Terser pass.

**Static sizes, not a ranking.** They say nothing about speed, and a port's size reflects how it
was written as much as the compiler.

## Negative controls

Reproduce with temporary copies of a port, a temporary registry naming them
(`{ "id", "language", "dir" }` entries) and `--receipts <tmp>` (build work directories then go to
`<tmp>/work`), so the real receipts never change. Each of these must be refused or exposed:

| Control (a pilot port copy) | Result |
| --- | --- |
| wrong on one non-bench case (`ties`) | `check.mjs`: fail; `bench.mjs`: `parity-not-passed`; the report prints no ratio |
| returns its cached result when called again with the same input object | `check.mjs`: fail at the `rewritten` step; `bench.mjs`: `parity-not-passed` |
| caches by input content in a module-level map (passes every black-box check) | with the library's bench variant, timed honestly (no cache hit can occur); `module state` flags the map; without a variant it would time the cache (port/js 0.02), hence the no-memoisation rule |
| right for its first 100 calls, wrong after (passes the checker) | `bench.mjs`: `output-mismatch` from in-sample verification, no ratio |
| right for its first 200 calls, then never returns | `bench.mjs`: `hang` after `--call-timeout` |
| never returns on one case | `check.mjs`: `hang` after `--call-timeout` in Node and in each browser (the browser is closed from outside and the session reopened for the next port) |
| Rust port that panics on one case | the case fails fast (`rust panic at src/pilot.rs` thrown), not a hang |
| loads the reference package through `import()` in an indirect-eval string | Node `check.mjs`: fail (`runtime guard: module load …`); browsers cannot see it, but `bench.mjs` refuses the port everywhere (`runtime-guard`); `eval refs` counts it |
| caches the scan of the text before its last word (exact) | under the pilot's rotating variant no hit is possible (port/js 0.68 Node / 0.66 Chrome, its own single-pass algorithm); under the earlier append-only variant it timed 0.09 |
| keeps a content cache on `globalThis` (typed binding, no `%raw`) | `check.mjs`: fail (`new global __pilotCache`) everywhere; even with that global allow-listed, the disjoint per-arm variant ranges defeat it (port/js 1.14 / 1.17, A/A 0.99 / 0.74) |
| fires image, prefetch-link and iframe-fetch requests that land late | `check.mjs`: fail in Chrome and Firefox (`request /verify-…-probe`); the innocent port checked next passes (requests are charged to their own load) |
| imports a `data:` module through an indirect eval | `check.mjs`: fail in Node (`module load data:…`), Chrome and Firefox (`CSP refused script-src-elem data`) |
| calls `process.binding('fs')` in every call | `check.mjs`: fail in Node (`process.binding(fs)`); `bench.mjs`: refused in every environment (`runtime-guard`) |
| reference adapter reading a symlinked data file at run time | `check.mjs`: `error` in Node (the reference reached `process.getBuiltinModule(node:fs)`); the symlink's target content is in the binding, so a data change makes receipts stale |
| a library whose reference output depends on the engine (`Math.log`) | parity passes per environment (three different reference digests in Node, Chrome, Firefox); `bench.mjs` gates each environment on its own digest and times it |
| Unicode `toLowerCase` instead of ASCII folding | `check.mjs`: fail; `bench.mjs`: `parity-not-passed` |
| check receipt made before the corpus, the reference adapter, a module the reference loads with `import()`, or the output form changed | report: `stale`, nothing counted, no ratio; `bench.mjs`: `parity-not-passed` (stale) |
| check receipt from another Node major, or a browser major other than the one being timed | report: `stale`; `bench.mjs`: `parity-not-passed` (stale) |
| `dist/index.min.js` edited, a file added to `dist/`, or a source edited after the build | `check.mjs` and `bench.mjs`: `stale`, exit 1; `sizes.mjs` and `matrix.mjs`: exit 1, nothing written |
| a syntax error; a build past `--timeout` | build receipt `built: false` (exit 1; exit 124 with `timedOut`); consumers report `build-failed` |

Writing such controls: the ReScript builder refuses a direct `eval` (Rollup's EVAL warning fails the
build), and Terser removes a call to a side-effect-free infinite loop, so a hang control's loop
must write somewhere (to a float: an int32 counter wraps negative within seconds and ends the loop). `tools/selftest.mjs` covers identity and content caches, argument mutation,
hot-path divergence, output-form loopholes, the network guard, literal and computed dynamic imports
and the source statistics on synthetic modules, and the page realm of browser cells.

## Per-language notes for faithful ports

Found while writing the pilot; each one decides whether a library port reproduces JS behaviour.

- **Rust (mires).** The runtime shim keeps a content-keyed memo of host-string conversions
  (`__rt._str_memo`, 4 MB budget), so a fixed bench input can reuse a conversion across calls; a
  bench variant removes that advantage (on the pilot the difference was within noise). `no_std` + `alloc`; `std::collections::HashMap` is unavailable (hashbrown cannot
  be compiled by this backend). `alloc::collections::BTreeMap` compiles and links (d3-hierarchy's
  `stratify` keys its nodes by id in one), but it iterates in key order, so where JS relies on
  insertion order maps are `mires_js::collections::HashMap` (a host `Map`, insertion-ordered). Strings
  cross as host strings read into Rust UTF-8 (`read::<&str>()`). `core::fmt` float formatting
  (`{:.3}`) pulls in `flt2dec` (Grisu with the Dragon fallback: 24 KB of the pilot's 98 KB
  program, per the optimizer report) and rounds exact ties to even (`{:.2}` of 0.125 is `0.12`;
  `toFixed` gives `0.13`). `sort_by` is stable. Export names of three letters trigger a mires
  minification warning, hence `matrix_run`.
- **ReScript.** `int` is a signed 32-bit type, but `*` compiles to `(a * b) | 0`, which loses bits
  once the exact product passes 2^53: 32-bit hashing needs `Math.Int.imul`. `Int.shiftRightUnsigned(x, 0)`
  is folded away (an `int` cannot hold the unsigned value), so unsigned output goes through a float.
  `Float.toFixed` is JS `toFixed`. Strings are JS strings (UTF-16); code points via `codePointAt`.
- **Melange.** JS strings, and `int` multiply emits `Math.imul`. Using any OCaml stdlib module
  (List, Hashtbl, Printf) imports emitted stdlib modules, so the builder emits the stdlib
  (`emit_stdlib true`, unlike the import-free comparator port) and bundles it: 19 KB of the
  pilot's 23 KB bundle before Terser is emitted stdlib, `List` alone 10 KB. `Js.Float.toFixed`, `Js.Map`, `Js.String.codePointAt` exist; there is no
  `padStart` binding.
- **js_of_ocaml.** Strings are OCaml byte strings: `Js.to_string` converts UTF-16 to UTF-8 (so ports
  work on UTF-8 bytes) and `Js.string` back. Native OCaml `int` is 63-bit but jsoo's is 32-bit: use
  `Int32` for portable 32-bit arithmetic. `Printf "%f"` uses JS `toFixed` up to 100 digits and
  below 1e21 (exact half-even beyond), and prints negative zero as `-0.000`, which `toFixed` does
  not. The runtime floor is 67 KB min (21 KB gzip).
- **Scala.js.** `f"$x%.3f"`, `getBytes(UTF_8)` and Scala collections are idiomatic but large
  (the pilot is 193 KB min, the floor 11 KB). Int arithmetic is 32-bit (`Math.imul`), `%08x` prints
  the unsigned value, `sortBy` is stable; avoid `Long` (emulated). Strings are UTF-16.
- **Dart.** On the web an `int` is a JS number: `hash * 0x01000193` passes 2^53 and loses bits, so
  32-bit multiplies must be split (`(x << 24) + x * 403` for FNV). `List.sort` is not stable: add an
  explicit last key. `utf8.encode`, `runes` and `toStringAsFixed` (JS `toFixed`) behave as in JS.
  dart2js cannot emit ES modules (global-exports facade).
- **Kotlin/JS.** No fixed-decimal formatter in common Kotlin (`String.format` is JVM-only);
  `roundToInt` rounds ties up, so `round(x * 1000)` matches `toFixed` inside the parity domain.
  `String.codePointAt` is JVM-only: code points are assembled from surrogate pairs. `UInt` gives
  32-bit unsigned arithmetic. `sortedWith` is stable; `LinkedHashMap` keeps insertion order. DCE is
  excellent (the floor is 46 bytes). The 2.4.10 IR compiler's output is reproducible only with a
  constant JVM identity hash (see "Reproducibility").
- **Elm.** No mutation, no typed arrays or `Uint8Array` across ports (bytes travel as JSON arrays of
  numbers or strings), no regex in `elm/core` (`elm/regex` is a JS `RegExp` wrapper), no
  fixed-decimal formatting and no hex printing in core; `Int` is a JS number, so 32-bit multiplies
  must be split and wrapped with `Bitwise.shiftRightZfBy 0`. `Dict` is ordered by key, not
  insertion: keep a first-occurrence index. All I/O and every host API goes through the JS host and
  ports, which are asynchronous in Elm (the wrapper's patch makes responses synchronous); a library
  needing browser APIs will be mostly JS host, and is labelled so above 50%.
- **GopherJS.** Strings are UTF-8; JS strings convert at the boundary (a lone high surrogate
  swallows the next code unit, a known GopherJS defect, `docs/COMPARATORS.md`), so corpora must be
  well-formed UTF-16. `int` is 32-bit and `int64`/`uint64` are emulated (two 32-bit halves, slow).
  `strconv.FormatFloat` rounds ties to even (0.125 at two digits is `0.12`). A float converted to
  `int` or `int32` lowers to `x >> 0`, ECMAScript ToInt32 (wrapping modulo 2^32, NaN to 0), and
  GopherJS's `math.Trunc` is `Copysign(float64(int(x)), x)`, so it is wrong for |x| ≥ 2^31 (a
  comparator finding, met again in long). Truncate with `math.Floor` or `math.Ceil` by sign, which
  call JS `Math.floor`/`Math.ceil`, and reduce with `math.Mod` (JS `%`, exact) before converting to
  an integer.
  `sort.SliceStable` needs reflection (the pilot uses it), and `fmt` is large (the pilot uses `strconv` instead). `hash/fnv` is
  available. Goroutines map to GopherJS's scheduler; the comparator ports avoid them for host async.

  From the MiniSearch port (repros in `build/library-matrix/gopherjs-bugs/`):
  - GopherJS treats every call through a function value or interface method as possibly blocking,
    so the caller and all its callers become `$restore` state machines. This covers
    `sort.SliceStable`, `slices.SortStableFunc`, `sort.Interface` and closures; avoid them on hot
    paths.
  - Under `-m`, `&slice[i]` of a struct element is about 20× slower (`$indexPtr` runs an
    `Object.defineProperty` on every call).
  - `strconv` adds about 53 KB.
- **Everyone.** JS `<` on strings compares UTF-16 code units, while Rust, Go and jsoo (UTF-8 bytes)
  compare code points, which differs for astral against U+E000–U+FFFF characters; JS regex
  semantics (`\w`, `\s`, `u` and `v` flags, lookbehind, case-insensitive Unicode matching) exist
  natively only in the families that bind `RegExp` (ReScript, Melange, jsoo, Scala.js, Dart, Kotlin,
  GopherJS through `js`), while Rust needs a regex crate or a host binding and Elm has only
  `elm/regex`. Unicode case mapping (`toLowerCase`) depends on each runtime's Unicode tables; the
  pilot avoids it, and its negative control is exactly that bug.
