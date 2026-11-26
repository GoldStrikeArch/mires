# Other-language comparators

This repository includes eight non-Rust compiler families ported to the same FingerprintJS workloads so Mires output can be checked and measured alongside existing compile-to-JS toolchains. Each family has a public OSS port under `experiments/fingerprintjs-<family>/` and a private Pro port under `experiments-pro/fp-pro-<family>/` (ignored by Git).

Offline builds and checks cover deterministic kernels, seed boundaries, package shapes, and static transfer sizes. They are not a full multi-browser equivalence or execution-time study. Archived `STATUS.md`, `VERDICT.md`, and older comparison tables inside individual port directories are historical snapshots and should not be read as fresh runs.

## Families and toolchains

| Family | Compiler toolchain | OSS variants | Pro build |
| --- | --- | --- | --- |
| `rescript` | ReScript 12.3.0 | Default ReScript + Rollup/Terser | Default |
| `melange` | Melange 7.0.1-53, OCaml 5.3.0 | Default Melange + Rollup/Terser | Default |
| `jsoo` | js_of_ocaml 6.4.1, OCaml 5.3.0 | Whole-program `--opt=3` with runtime attribution | Default |
| `scalajs` | Scala 3.8.4, Scala.js 1.22.0, Scala CLI 1.18.0 | `fast/` and `full/` | Release linker |
| `dart` | Dart SDK 3.13.7, `dart2js` | `o1/` and `o2/` | `-O2` |
| `kotlin` | Kotlin Toolchain 0.12.0; Gradle 9.4.1 / Kotlin plugin 2.4.10 | `readable/`, `proxy-production/`, `production/` | Toolchain JS application |
| `elm` | Elm 0.19.2 | `standard/` and compiler-aware-minified `production/` | Default |
| `gopherjs` | GopherJS 1.21.0, Go 1.21.13 | `gopherjs build -m` + Terser | See [GopherJS](#gopherjs) |

`dart` compiles Dart to JavaScript via `dart2js` (not Dart-to-Wasm). `jsoo` is `js_of_ocaml`, separate from Melange. `gopherjs` compiles Go with GopherJS (not `GOOS=js GOARCH=wasm`).

## Setup and reproduction

Run these commands from the repository root. Root npm dependencies are pinned in `package-lock.json`, and compiler caches and generated artifacts are gitignored.

```sh
npm ci

# macOS prerequisites for the OCaml and Scala families:
brew install opam scala-cli

# Local OCaml switch under .toolchains/opam (leaves your default opam switch untouched).
npm run setup:comparators:ocaml

# Local Go 1.21.13 + GopherJS 1.21.0 under .toolchains/go (leaves system Go untouched).
npm run setup:comparators:go

# Build the OSS TypeScript reference before packaging or checking ports.
npm run build:oss-reference

# Build all eight families (OSS and Pro).
npm run build:comparators

# Run artifact, deterministic kernel, numerical boundary, and Pro validation checks.
npm run check:comparators

# Record static transfer sizes (raw, gzip-9, Brotli-11).
node experiments/tools/measure-comparators.mjs
```

### Running a subset

```sh
python3 experiments/tools/run-comparator-builds.py \
  --languages rescript,scalajs,dart --suite oss

python3 experiments/tools/check-comparators.py \
  --languages rescript,scalajs,dart --suite oss

python3 experiments/tools/run-comparator-builds.py \
  --languages melange,jsoo --suite pro
```

Valid family names are `rescript`, `melange`, `jsoo`, `scalajs`, `dart`, `kotlin`, `elm`, and `gopherjs`. `--suite` accepts `oss`, `pro`, or `all`. The per-port build timeout defaults to 1,200 seconds (`--timeout` overrides it); a timed-out build fails without reusing prior output.

Individual port directories also support `npm run build`, though the common runner is required when you need build receipts and input/artifact digests.

## Toolchain notes

### Dart

Dart must be on `PATH`; the ports target SDK `^3.13.0` (tested with 3.13.7). Set `DART` to point at another compatible binary if needed, and record the version change when comparing results.

### Kotlin/JS

`experiments/tools/kotlin` is a version-pinned Kotlin Toolchain 0.12.0 bootstrap wrapper that fetches its own compiler and JDK dependencies. The OSS production build downloads Gradle 9.4.1 and verifies its SHA-256 before unpacking.

**Deterministic production builds.** The Kotlin/JS 2.4.10 IR compiler orders supertypes inside `initMetadataForClass(...)` by iterating identity-hashed sets, which could swap `[Collection, KtSet]` and `[KtSet, Collection]` for `AbstractMutableSet` and `HashSet` across clean builds. `gradle-production/gradle.properties` runs the compiler in-process with `-XX:+UnlockExperimentalVMOptions -XX:hashCode=2`. That JVM flag changes no compiler options and patches no output; it only pins the iteration order so clean builds are byte-identical. The library matrix's Kotlin template uses the same setting.

### Scala.js

Project directives pin Scala 3.8.4 and Scala.js 1.22.0 even if the installed Scala CLI defaults to a newer Scala version. Record the CLI version separately from the language and linker versions. Dependency downloads use Maven Central by default.

### OCaml (`melange` and `jsoo`)

`npm run setup:comparators:ocaml` creates a local opam root at `.toolchains/opam` with switch `mires-comparators` containing OCaml 5.3.0, Dune 3.24.2, Melange 7.0.1-53, js_of_ocaml 6.4.1, and Lwt 6.1.2. The full package solution is checked in at `experiments/ocaml-comparators.export` and imported by setup runs. Override locations or parallelism with `MIRES_OPAM_ROOT`, `MIRES_OPAM_SWITCH`, and `MIRES_BUILD_JOBS`.

Dune's automatic CPU detection crashed on macOS during setup, so the setup script, common runner, and OCaml port builders pass `DUNE_JOBS=4` explicitly. This keeps opam sandboxing enabled and leaves compiler behavior unchanged.

### GopherJS

GopherJS ports live at `experiments/fingerprintjs-gopherjs/` (OSS) and `experiments-pro/fp-pro-gopherjs/` (Pro).

`npm run setup:comparators:go` installs GopherJS 1.21.0 alongside Go 1.21.13 (`GOPHERJS_GOROOT`) under `.toolchains/go` and sets `GOCACHE=.toolchains/go/cache`, leaving any system Go installation and cache untouched. The Go tarball is verified against its go.dev SHA-256, and GopherJS dependencies are verified against the Go checksum database. Override the root with `MIRES_GO_ROOT`.

On current macOS, Go 1.21's internal linker omits the `LC_UUID` load command, which causes `dyld` to refuse binaries linked against CoreFoundation and Security (as GopherJS is; fixed in Go 1.22). Setup therefore links the GopherJS binary with `-linkmode=external` and re-signs it ad hoc after Go rewrites the binary. That changes only how the tool binary itself is linked, leaving emitted JavaScript untouched.

GopherJS offers only `-m` for minification, so each port has a single `dist/` output that serves as its production row. Because GopherJS does not emit ES modules, each program populates `js.Module.exports` and the build script wraps it in a loader binding `window` and `module` before re-exporting through an ESM/IIFE facade (matching `jsoo`).

**Emit policy and side effects.** GopherJS output ships unmodified except for the module loader wrapper (and, in the Pro build, renaming reused `s` loop labels apart to avoid a Rollup 3.30 tree-shaking bug where an inner function reusing a label drops the outer label—zero added minified bytes). Importing either GopherJS port therefore carries GopherJS's runtime side effects:

- `globalThis.fs` is defined;
- `Error.stackTraceLimit` is set to `Infinity`;
- if a page-global `require` exists, it is called with `"fs"` and `"util"`.

**Port summary and verified checks.** Both ports use idiomatic `uint64` Murmur hashing and JS Promises, `MessageChannel`, and timers (no goroutines), with zero embedded JavaScript (`eval`, `Function`, or product `.js` code; Pro has only a 12-line re-export wrapper):

- OSS: 3,644 lines of Go (`package main`). Passed package checks (3/3), kernels (58/58), seeds (477/477), strict static audit, SSR-safe import, and Chrome 158 / Firefox 160 differentials (0 normalized differences).
- Pro: 6,467 lines of Go across 22 files. Passed package checks (5/5), validation (all five error codes), ownership check (`target=6489 js=13 embedded=0`), SSR-safe import, and Chrome 158 / Firefox 160 strict differentials (0 differences across 144/144 and 145/145 fields).

Known coverage limits for GopherJS: Safari WebDriver was not run (requires enabling Safari's "Allow Remote Automation"); one timing-sensitive Pro signal was intermittently flaky under heavy host load on Firefox (passing 5/5 and 4/4 when idle); one Pro browser signal is stubbed on WebKit and Android (matching the Scala and Rust Pro ports; see `experiments-pro/fp-pro-gopherjs/VERDICT.md`). Remaining out-of-contract deviations documented in each port's `NOTES.md`/`VERDICT.md` are error-message wording, constructible async exports, accepting BigInt seeds, null-argument error classes, and the import side effects listed above.

Reference static sizes measured with `measure-comparators.mjs` (Node zlib, main module):

| Suite | Artifact | Raw bytes | gzip-9 | Brotli-11 |
| --- | --- | ---: | ---: | ---: |
| OSS | GopherJS `fp.esm.min.js` | 201,379 | 57,352 | 47,866 |
| OSS | GopherJS runtime floor (single-export empty program) | 42,619 | 14,111 | 12,707 |
| OSS | TypeScript oracle `fp.min.js` (for scale) | 36,746 | 16,344 | 14,239 |
| Pro | GopherJS `fp.esm.min.js` | 298,020 | 79,614 | 63,897 |

In the OSS `-m` output, the prelude accounts for ~42 KB and program code ~127 KB, with the `strings` package (and its `internal/abi` / `internal/reflectlite` transitive dependencies) adding ~62 KB.

**Reproduced GopherJS and Rollup defects.** Each of these was isolated with a minimal repro on this toolchain:

- `math.Trunc` routes through a 32-bit integer conversion: `Trunc(4294967295) == 1` and `Trunc(2**53) == 0`. `Modf(-0.5)` returns `+0` as its integer part.
- A lone high surrogate in a JS-to-Go string conversion swallows the following code unit (`"\ud800 x"` becomes U+2420, dropping the space), and a trailing lone high surrogate yields invalid UTF-8.
- A JS `throw null` passes through Go `defer` / `recover()` unseen (`recover()` returns `nil` and the function returns zero values).
- Rollup 3.30's tree-shaking drops an outer loop label reused by a nested function (`export function f(a){ s: while(true){ if(a) break s; return function(){ s: while(true){ if(a) break s; return } } } }`), which is why the Pro build renames GopherJS's labels apart.

## Production artifacts to compare

Several builders leave a readable or development variant at the root `dist/` path. Compare production rows using the specific paths below:

| Family | OSS production candidate |
| --- | --- |
| ReScript | `dist/fp.esm.min.js` |
| Melange | `dist/fp.esm.min.js` |
| js_of_ocaml | `dist/fp.esm.min.js` |
| Scala.js | `dist/full/fp.esm.min.js` |
| Dart | `dist/o2/fp.esm.min.js` |
| Kotlin/JS | `dist/production/fp.esm.min.js` |
| Elm | `dist/production/fp.esm.min.js` |
| GopherJS | `dist/fp.esm.min.js` |

Scala's root `dist/` alias is `fast-link`, Dart's is `-O1`, Kotlin's is the readable Toolchain variant, and Elm's is the standard-minifier variant. The Kotlin packaging proxy is a wrapper artifact and should not be compared as the Gradle optimizing build.

Pro packages live under their own `dist/` directories. Archived Pro port scopes and configurations are not automatically equivalent to the Rust Pro port, so passing offline checks do not imply universal feature parity across languages.

## What the offline checks verify

Build receipts hash each port's inputs before and after compilation along with every output artifact's digest. `check-comparators.py` rehashes every listed artifact before testing it and rejects any mismatch with `artifact-identity-mismatch`.

For each OSS variant (including default aliases), the checker runs:

- 58 comparisons against the built TypeScript reference covering Murmur hashing (Unicode, lengths, seeds), component hashing, and debug serialization;
- 477 seed-boundary comparisons covering signed/unsigned edges, fractional and non-finite inputs, wrapping, large integers, and pseudo-random seeds across nine inputs;
- package checks covering ESM syntax, importing without browser globals, exported symbol names, and declaration-file checksums.

For each Pro port, the checker tests readable and minified ESM imports, the eight public export names, the copied `.d.ts` contract, worker dependency closure, and five invalid-start configurations against a fake worker. It does not run real browser collection, real workers, or network requests. Because `.d.ts` files are copied contracts, a matching checksum verifies the file text and does not prove that inferred types match runtime behavior.

## Port fixes

The fixes below were made inside the non-Rust language ports (`mires` itself was unchanged), keeping the original reference expectations intact:

- **Scala.js seed wrapping:** `Double.toInt` saturated values above `i32::MAX`, failing seed `0xffffffff`. Replaced with truncation, modulo `2^32` wrapping, and signed mapping in both OSS and Pro's embedded OSS Scala code.
- **Kotlin seed wrapping:** Applied the same wrapping fix to Kotlin's OSS and embedded Pro seed conversion.
- **Dart non-finite seeds:** Calling `.toInt()` on `NaN` or infinities threw in the OSS adapter; non-finite numeric seeds now normalize to zero before conversion.
- **Elm fractional seeds:** Fractional seeds failed Elm's integer decoder and collapsed to zero. The JS-to-Elm bridge now coerces seeds to 32-bit integers before crossing into Elm while leaving the Elm hashing logic untouched.
- **Build portability:** Updated root npm dependency resolution, local opam switch selection, version-pinned tool wrappers, and Kotlin 0.12's module output discovery (picking the application module instead of the first `.mjs` file).

## Receipts, sizing, and caveats

Running the build, check, and size scripts writes receipts under `build/comparators/`:

```text
build/comparators/*.build.json
build/comparators/*.package.json
build/comparators/*.kernels.json
build/comparators/*.seeds.json
build/comparators/checks-all-*.json
build/comparators/sizes.json
build/comparators/logs/
build/comparators/attempts/
```

`sizes.json` records raw, gzip-9, and Brotli-11 sizes using Node's `zlib`, compressing separately fetched resources individually. A worker that imports the main module does not double-count the main module transfer, while a separately bundled worker (such as ReScript Pro's) is counted as its own transfer.

Treat static sizes as raw data, not a ranked comparison across languages. Do not mix these numbers with system-`gzip` figures, unrelated timing runs, or different upstream package versions without labeling the difference. Note also that the Elm OSS port includes a substantial handwritten JavaScript host and is not a pure-Elm bundle.

## Private files and npm advisories

`experiments-pro/` is gitignored and holds the private Pro reference and historical Pro ports.

The pinned packaging dependencies in `package-lock.json` carry seven known npm audit advisories (two low, two moderate, three high; recorded in `build/comparators/npm-audit.json`). Updating them is tracked separately from the pinned comparator baseline; run these local build tools only on trusted inputs.
