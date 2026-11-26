# Restored compiler-comparison implementations

## Current status

On 2026-11-08, all seven archived non-Rust implementations were restored for both FingerprintJS OSS and Fingerprint Pro: **14 fresh builds pass**. All current offline checks pass. This is a restored and runnable comparison suite, not a completed browser-equivalence or runtime-performance study.

The current Rust compiler source was not changed by this restoration: 246 files under its compiler/runtime crates and patches match their pre-restoration hashes. Original archives remain untouched.

## Source layout

Each family has an OSS directory `experiments/fingerprintjs-<family>/` and a Pro directory `experiments-pro/fp-pro-<family>/`.

| Family | Compiler used | OSS variants restored and built | Pro build |
| --- | --- | --- | --- |
| `rescript` | ReScript 12.3.0 | Default ReScript + Rollup/Terser output | Pass |
| `melange` | Melange 7.0.1-53, OCaml 5.3.0 | Default Melange + Rollup/Terser output | Pass |
| `jsoo` | js_of_ocaml 6.4.1, OCaml 5.3.0 | Whole-program `--opt=3`, plus retained runtime attribution | Pass |
| `scalajs` | Scala 3.8.4, Scala.js 1.22.0, Scala CLI 1.18.0 | `fast/` and `full/` | Pass, release linker |
| `dart` | Dart SDK 3.13.7, dart2js | `o1/` and `o2/` | Pass, `-O2` |
| `kotlin` | Kotlin Toolchain 0.12.0; Gradle 9.4.1 / Kotlin plugin 2.4.10 | `readable/`, `proxy-production/`, `production/` | Pass, Toolchain JS application |
| `elm` | Elm 0.19.2 | `standard/` and compiler-aware-minified `production/` | Pass |
| `gopherjs` | GopherJS 1.21.0, Go 1.21.13 | Default: `gopherjs build -m` + Terser (added 2026-11-18) | See [GopherJS](#gopherjs-added-2026-11-18) |

The family name `dart` means Dart-to-JavaScript, not Dart-to-Wasm. The `jsoo` family is js_of_ocaml, distinct from Melange. The `gopherjs` family is Go compiled by GopherJS, not Go's own `GOOS=js GOARCH=wasm` target.

The original source archive is `experiments-2026-09-05/p14-prebuilt-current/source/source.tar.gz`, SHA-256 `3aca257b220d0bbdb2940aba9c2b4f86456097d3c531266547aeb0f6451f437d`. It supplied 1,155 files across the 14 ports. A further 34 files under `experiments/fingerprintjs-compare/` restore shared comparison support and historical records.

The source versions are historical. The current build tools and narrow implementation corrections are described below. Do not treat the archived STATUS/VERDICT files or old comparison tables as fresh results; those documents now carry a historical-record notice.

## Install and reproduce

Run from the repository root. Root npm dependencies are pinned in `package-lock.json`; compiler caches and generated output are ignored by Git.

```sh
npm ci

# macOS prerequisites used for the OCaml and Scala families:
brew install opam scala-cli

# Local OCaml switch; does not change the user's default opam switch or shell startup files.
npm run setup:comparators:ocaml

# Local Go 1.21.13 + GopherJS 1.21.0 under .toolchains/go; does not use or change the system Go.
npm run setup:comparators:go

# The OSS TypeScript reference must exist before packaging/comparing its ports.
npm run build:oss-reference

# Build all eight families, OSS and Pro.
npm run build:comparators

# Exact built-artifact checks, deterministic kernels, numerical boundaries, and Pro validation.
npm run check:comparators

# Static transfer sizes, not execution-time measurements.
node experiments/tools/measure-comparators.mjs
```

Dart must be available on PATH; this machine already had Dart 3.13.7. The archived projects require SDK `^3.13.0`. `DART` can name another compatible executable, but record that change as a new toolchain configuration.

`experiments/tools/kotlin` is the version-pinned Kotlin Toolchain 0.12.0 bootstrap wrapper, copied with its original checksums and copyright. It downloads its own compiler/JDK dependencies as needed. The OSS production builder downloads Gradle 9.4.1 and verifies the archive against its published SHA-256 before unpacking it. Existing archives and generated outputs are not a substitute for a successful current build.

**Kotlin production determinism (2026-11-19).** The Kotlin/JS 2.4.10 IR compiler orders the supertypes in `initMetadataForClass(...)` by iterating identity-hashed sets. So the Gradle production emit could flip the order of two interface lists between builds: `[Collection, KtSet]` vs `[KtSet, Collection]` for `AbstractMutableSet` and `HashSet`. The size stayed the same.

`gradle-production/gradle.properties` now runs the compiler in-process with a constant JVM identity hash (`-XX:+UnlockExperimentalVMOptions -XX:hashCode=2`). This changes no compiler setting and patches no emit; it only fixes which of the compiler's own orders is produced.

With the setting, three clean builds were byte-identical, and the OSS and Pro Kotlin checks pass 12/12. Against the Nov-8 artifact, the production output differs only in those two reordered lists: raw bytes are unchanged and gzip-9 moved by +1 byte (29,232). The Toolchain variants and the Pro port were already stable: unchanged across clean builds and identical to their recorded artifacts. The library matrix's Kotlin template uses the same setting.

Scala's project directives pin Scala 3.8.4 and Scala.js 1.22.0 even though the installed Scala CLI's default Scala version is newer. Record the CLI version separately from the language/linker versions. Dependency downloads use the central Maven repository by default through the common runner.

### Run a smaller selection

```sh
python3 experiments/tools/run-comparator-builds.py \
  --languages rescript,scalajs,dart --suite oss

python3 experiments/tools/check-comparators.py \
  --languages rescript,scalajs,dart --suite oss

python3 experiments/tools/run-comparator-builds.py \
  --languages melange,jsoo --suite pro
```

Supported family names are `rescript,melange,jsoo,scalajs,dart,kotlin,elm,gopherjs`. The suite is `oss`, `pro`, or `all`. A build timeout defaults to 1,200 seconds per port and can be set with `--timeout`. Timed-out builds fail; old output is not accepted as a pass.

Individual port builders also work from their directories using `npm run build`. Use the common runner when results need build receipts and source/artifact identities.

### OCaml isolation and reproducibility

The setup script uses `.toolchains/opam`, switch `mires-comparators`, OCaml 5.3.0, Dune 3.24.2, Melange 7.0.1-53, js_of_ocaml 6.4.1, and Lwt 6.1.2. `experiments/ocaml-comparators.export` records the complete installed package solution and is imported by subsequent setup runs.

`MIRES_OPAM_ROOT`, `MIRES_OPAM_SWITCH`, and `MIRES_BUILD_JOBS` permit explicit overrides. The checked setup was also rerun successfully against the exported solution.

Dune's automatic CPU/job detection crashed on this Mac. Explicit `DUNE_JOBS=4` avoids that path and is set by the setup/common runner and direct OCaml builders. The failed initial installation/build logs are retained. The workaround does not disable opam's sandbox and does not change compiler semantics.

### GopherJS (added 2026-11-18)

GopherJS is the eighth family, added after the restoration as a new port rather than a restored one. Its OSS port is `experiments/fingerprintjs-gopherjs/` and its Pro port is `experiments-pro/fp-pro-gopherjs/`.

The toolchain is GopherJS 1.21.0, the latest release (2026-06-26). It compiles Go 1.21 and needs a Go 1.21 distribution for the standard library (`GOPHERJS_GOROOT`), so the setup script installs Go 1.21.13 next to it under `.toolchains/go`. The system Go is never used. The Go archive is verified against its go.dev SHA-256. GopherJS and its module dependencies are verified by the Go checksum database. `MIRES_GO_ROOT` overrides the location.

On current macOS, Go 1.21's internal linker produces binaries without `LC_UUID`. dyld refuses such a binary when it links system frameworks ("missing LC_UUID load command"); GopherJS links CoreFoundation and Security, while a pure-Go binary still runs. Go fixed this only from 1.22. The script therefore links GopherJS with the system linker (`-linkmode=external`) and re-signs it ad hoc, because Go rewrites the binary after the linker's signature. This changes how the GopherJS tool itself is linked, not the JavaScript it emits.

GopherJS has no optimisation levels beyond `-m`, so each port has one variant: the root `dist/`, which is also the production row. It cannot emit ES modules. The program fills GopherJS's `js.Module.exports`, and the build script wraps the emit in a function that binds `window` and `module`, then re-exports through the ESM / IIFE facade, much as for js_of_ocaml.

**Emit policy (both ports).** The GopherJS emit ships untouched; only the packaging needed to load it is added. As a result, importing either port has GopherJS's runtime side effects, which each port documents:
- `globalThis.fs` is defined;
- `Error.stackTraceLimit` becomes `Infinity`;
- if a page-global `require` exists, it is called with `fs` and `util`.

The one transformation is in the Pro build: it renames GopherJS's reused `s` loop labels apart. That works around Rollup 3.30's tree-shaking, which drops an outer label when a nested function reuses its name. It is a pure renaming and adds 0 bytes to the minified output.

Go builds use `GOCACHE=.toolchains/go/cache`, so the user's global Go build cache is not used.

**Results (2026-11-18), independently verified.** Two Go ports were written:

| Port | Size | Structure | Embedded JavaScript |
| --- | --- | --- | --- |
| OSS | 3,644 lines of Go | `package main` | None: no `eval`, no `Function`, no product `.js` |
| Pro | 6,467 lines of Go, 22 files | — | None (`maxEmbeddedJsLines: 0`; the only `.js` is a 12-line re-export) |

Both use idiomatic `uint64` Murmur. All async work runs on JS Promises, `MessageChannel` and timers; neither uses goroutines.

Checks that passed:
- Fresh runner builds of both ports, deterministic: the clean-room toolchain and repeated builds give byte-identical `dist/`.
- OSS: package 3/3, kernels 58/58, seeds 477/477, on both the minified and the readable ESM.
- Pro: package 5/5, validation PASS (all five error codes), and ownership `PASS target=6489 js=13 embedded=0`.
- SSR-safe import for both.
- Chrome 158 and Firefox 160:
  - the OSS differential, plus `run-feature-parity`, shows 0 normalized differences;
  - the Pro strict differential shows 0 differences, 0 missing signals and 0 value mismatches, with 144/144 and 145/145 fields.
- The strict OSS static audit gives contract PASS, strict PASS and 0 metadata differences.
- `check-comparators.py --suite all`: 67/67 across all eight families.

Not covered:
- **Safari** was not run: WebDriver needs Safari's "Allow Remote Automation" setting, which is off on this machine.
- **One timing-sensitive Pro signal is flaky on Firefox.** In one window GopherJS passed 4 of 10 attempts and the interleaved Scala.js control 3 of 5, which is consistent with host load (inferred, not measured). Both a later idle window (5/5) and the final build (4/4) passed.
- **One Pro browser signal is stubbed on WebKit and Android**, as in the Scala and Rust Pro ports. A Chrome/Firefox differential cannot see this. Details are in the private `experiments-pro/fp-pro-gopherjs/VERDICT.md`.

Each port's `NOTES.md` / `VERDICT.md` lists its remaining observable deviations, all outside the checked contract:
- error-message wording;
- constructible async exports;
- a BigInt seed being accepted;
- null-argument error classes;
- the import side effects above.

Sizes, by the same method as `measure-comparators.mjs` (Node zlib), main module:

| Suite | Artifact | Bytes | gzip-9 | Brotli-11 |
| --- | --- | ---: | ---: | ---: |
| OSS | GopherJS `fp.esm.min.js` | 201,379 | 57,352 | 47,866 |
| OSS | GopherJS runtime floor (one-export empty program, `fp.min.js` shape) | 42,619 | 14,111 | 12,707 |
| OSS | TypeScript oracle `fp.min.js`, for scale | 36,746 | 16,344 | 14,239 |
| Pro | GopherJS `fp.esm.min.js` | 298,020 | 79,614 | 63,897 |

Among the OSS production rows, only Scala.js `full` (229,475 / 68,148) is larger than GopherJS. js_of_ocaml (136,633 / 48,435) and Dart `-O2` (152,185 / 52,634) are the nearest. Among the Pro rows, GopherJS is about the size of Scala.js (296,145 / 84,827) and the largest raw.

Of the OSS `-m` output, the prelude is about 42 KB and the program 127 KB. The `strings` package and what it links (`internal/abi`, `internal/reflectlite`) add about 62 KB. These are static sizes, not a qualified ranking.

**GopherJS defects found** (each reproduced with a minimal program built by this toolchain; candidates for upstream reports):
- `math.Trunc` goes through a 32-bit integer path: `Trunc(4294967295) == 1`, `Trunc(2**53) == 0`. Also, `Modf(-0.5)` returns +0 as the integer part.
- A lone high surrogate in a JS → Go string conversion swallows the next code unit (`"\ud800 x"` → U+2420, the space lost). At the end of a string it produces invalid UTF-8.
- A JS `throw null` passes through Go `defer` / `recover()` unseen: `recover()` returns nil and the function returns zero values.

Rollup 3.30's tree-shaking drops an outer label that a nested function reuses (minimal repro: `export function f(a){ s: while(true){ if(a) break s; return function(){ s: while(true){ if(a) break s; return } } } }`). The Pro build renames GopherJS's labels apart for this reason.

## Which output to compare

Some archived builders deliberately retain their readable/development variant as the root `dist/` alias. Do not accidentally compare it against another compiler's production variant.

| Family | Main OSS production-comparison candidate |
| --- | --- |
| ReScript | `dist/fp.esm.min.js` |
| Melange | `dist/fp.esm.min.js` |
| js_of_ocaml | `dist/fp.esm.min.js` |
| Scala.js | `dist/full/fp.esm.min.js` |
| Dart | `dist/o2/fp.esm.min.js` |
| Kotlin/JS | `dist/production/fp.esm.min.js` |
| Elm | `dist/production/fp.esm.min.js` |
| GopherJS | `dist/fp.esm.min.js` |

Scala's root alias is fast-link; Dart's is `-O1`; Kotlin's is the toolchain/readable variant; Elm's is the standard-minifier variant. Their additional variants are all built and checked. The Kotlin packaging proxy is not presented as the Gradle optimizing compiler.

The Pro packages live under their own `dist/` directories. Their archived implementation scopes and configurations are not automatically equivalent to the later Rust Pro implementation. Current passing checks do not establish universal cross-language product parity.

## Fresh checks performed

The final build receipts bind authored port inputs and selected shared build inputs before and after compilation, plus every delivered artifact's digest. All 14 builds had unchanged inputs. The checker rehashes all listed artifacts before using them. A negative control changed an expected artifact digest in a temporary receipt: checking failed with `artifact-identity-mismatch`, and the original receipt was restored. No actual package artifact was altered by that test.

The final offline run passes **62 check commands** across **16 OSS artifact variants and seven Pro artifacts**. Default aliases are included, so this is seven OSS implementations, not 16 independent implementations.

For each OSS artifact variant:

- 58 comparisons against the exact built TypeScript reference: Murmur hashing over Unicode/length/seed cases, component hashing, and debug serialization.
- 477 seed-boundary comparisons: signed/unsigned boundaries, fractional and non-finite inputs, wrapping, large integers, and deterministic pseudo-random seeds over nine inputs.
- Packaged module syntax, import without browser globals, export names, and declaration-file checksum.

That is 928 standard comparisons plus 7,632 seed comparisons: **8,560 deterministic reference comparisons** across aliases and variants, all passing.

For every Pro port, the checks cover ordinary/minified ESM imports, the exact eight public export names, copied declaration contract, worker dependency closure, and five invalid-start-configuration cases using a fake worker. They do not call valid collection, start real workers, or send requests. In total, the package checker records 83 passing assertions across OSS/Pro artifacts.

Declarations in these ports are copied contracts. A matching checksum does not prove inferred declarations match every language implementation behavior.

## Repairs made during restoration

All compiler source repairs here are in the language ports, not in `mires`.

**Scala.js seed wrapping.** The archived `Double.toInt` seed conversion saturated values above the signed range; seed `0xffffffff` failed all 12 original string vectors. The replacement truncates, wraps modulo 2^32, and maps into the signed range before conversion. Both OSS and Pro's embedded OSS Scala source were corrected.

**Kotlin seed wrapping.** The analogous Kotlin conversion had the same boundary problem. Both OSS and embedded Pro source now preserve the reference's wrapping behavior.

**Dart non-finite seeds.** Calling `.toInt()` on non-finite numeric seeds threw in the exported OSS adapter. It now normalizes those values to zero before conversion.

**Elm port protocol.** Fractional seeds failed the integer decoder and became zero. The existing JavaScript-to-Elm bridge now explicitly coerces the seed to its 32-bit integer representation; the hashing algorithm remains Elm.

The original expectations were not relaxed. Failed receipts are retained under `build/comparators/attempts/scala-seed-before/` and `build/comparators/attempts/numeric-boundaries-before/`.

Build portability repairs include root npm dependency lookup, local OCaml switch selection, version-pinned tool entry points, and Kotlin 0.12's moved output location. Kotlin entry discovery now selects the actual application module rather than the first arbitrary `.mjs` file. Original numeric implementation files remain recoverable from the archive and original-hash manifest.

## Results, sizing, and evidence

Current outputs:

```text
build/comparators/*.build.json
build/comparators/*.package.json
build/comparators/*.kernels.json
build/comparators/*.seeds.json
build/comparators/checks-all-rescript-melange-jsoo-scalajs-dart-kotlin-elm.json
build/comparators/sizes.json
build/comparators/logs/
build/comparators/attempts/
```

The size report contains 23 package/profile rows. It records raw, gzip, and Brotli bytes using the recorded Node compression versions, compressing separately fetched resources separately. A worker importing the main module does not incur a second copy of the main transfer in that graph, but its main dependency is not omitted. The ReScript Pro worker is separately bundled and counted as such.

These static sizes are not a qualified language ranking. Do not combine them with old system-gzip measurements, unrelated runtime timings, or different upstream versions without labeling those differences.

No three-browser differential matrix or quiet-machine timing protocol was run. Browser collection, async lifecycle, full source ownership, failure branches, and the Pro implementation-completeness gaps still require their own qualification. The Elm OSS implementation explicitly contains a substantial JavaScript host and must not be advertised as an all-Elm implementation.

## Privacy, dependencies, and backups

`experiments-pro/` remains ignored by Git. It contains the private retained Pro reference, partial original source, and historical ports. The Pro reference module still matches SHA-256 `40c43c3afde41f9dbc3b15ac9f11dd4d3593fe4522f37ae22b04de6ea95144f9`. Nothing was published.

The existing pinned packaging dependencies report seven npm audit findings: two low, two moderate, and three high. The report is saved at `build/comparators/npm-audit.json`. A breaking automatic dependency upgrade was not mixed into the restored benchmark baseline. Use the local suite with trusted inputs; resolving those packaging dependency advisories is separate maintenance work requiring a rebuild/recheck.

The pre-restoration configuration and compiler identity were backed up under `/Users/mikhail.pertsev/mires-recovery-backups/comparators-20261108T032853Z/`. Final source/evidence archives are recorded there separately for public OSS tooling and private Pro material. These are same-device backups, not protection against loss of the machine.
