# Restored compiler-comparison implementations

## Current status

On 2026-11-08, all seven archived non-Rust implementations were restored for both FingerprintJS OSS and Fingerprint Pro: **14 fresh builds pass**. All current offline checks pass. This is a restored and runnable comparison suite, not a completed browser-equivalence or runtime-performance study.

The current Rust compiler source was not changed by this restoration: 246 files under its compiler/runtime crates and patches match their pre-restoration hashes. No commit or push was made. Original archives and the prior v2 recovery remain untouched.

Machine-readable summaries:

- [Restoration and check status](comparator-restoration-status.json)
- [Observed toolchains and configured pins](comparator-toolchains.json)
- [Original restored source hashes](comparator-restored-sources.json)

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

The family name `dart` means Dart-to-JavaScript, not Dart-to-Wasm. The `jsoo` family is js_of_ocaml, distinct from Melange.

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

# The OSS TypeScript reference must exist before packaging/comparing its ports.
npm run build:oss-reference

# Build all seven families, OSS and Pro.
npm run build:comparators

# Exact built-artifact checks, deterministic kernels, numerical boundaries, and Pro validation.
npm run check:comparators

# Static transfer sizes, not execution-time measurements.
node experiments/tools/measure-comparators.mjs
```

Dart must be available on PATH; this machine already had Dart 3.13.7. The archived projects require SDK `^3.13.0`. `DART` can name another compatible executable, but record that change as a new toolchain configuration.

`experiments/tools/kotlin` is the version-pinned Kotlin Toolchain 0.12.0 bootstrap wrapper, copied with its original checksums and copyright. It downloads its own compiler/JDK dependencies as needed. The OSS production builder downloads Gradle 9.4.1 and verifies the archive against its published SHA-256 before unpacking it. Existing archives and generated outputs are not a substitute for a successful current build.

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

Supported family names are `rescript,melange,jsoo,scalajs,dart,kotlin,elm`. The suite is `oss`, `pro`, or `all`. A build timeout defaults to 1,200 seconds per port and can be set with `--timeout`. Timed-out builds fail; old output is not accepted as a pass.

Individual port builders also work from their directories using `npm run build`. Use the common runner when results need build receipts and source/artifact identities.

### OCaml isolation and reproducibility

The setup script uses `.toolchains/opam`, switch `mires-comparators`, OCaml 5.3.0, Dune 3.24.2, Melange 7.0.1-53, js_of_ocaml 6.4.1, and Lwt 6.1.2. `experiments/ocaml-comparators.export` records the complete installed package solution and is imported by subsequent setup runs.

`MIRES_OPAM_ROOT`, `MIRES_OPAM_SWITCH`, and `MIRES_BUILD_JOBS` permit explicit overrides. The checked setup was also rerun successfully against the exported solution.

Dune's automatic CPU/job detection crashed on this Mac. Explicit `DUNE_JOBS=4` avoids that path and is set by the setup/common runner and direct OCaml builders. The failed initial installation/build logs are retained. The workaround does not disable opam's sandbox and does not change compiler semantics.

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
