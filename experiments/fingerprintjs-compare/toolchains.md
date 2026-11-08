# Pinned toolchains

Recorded 2026-08-14 on this machine. Wave 1 agents must not invent a different
switch or compiler pin.

| Tool | Version | Notes |
| --- | --- | --- |
| Node.js | v24.5.0 | ReScript wants ≥ 22 |
| npm | 11.9.0 | |
| ReScript | 12.3.0 | via `npx --yes rescript@12.3.0` |
| rustc | 1.97.1 (`nightly-2026-07-27` pin in the rust experiment) | `experiments/fingerprintjs-rust/rust-toolchain.toml` |
| `rustc-codegen-js` | `spike/rustc-codegen-js` | Frozen for the ports; do not patch |
| Oracle Rollup / Terser | the versions in `experiments/fingerprintjs/node_modules` | Reuse them; do not add a second Terser |
| opam | 2.5.2 (Homebrew) | Initialized with `--disable-sandboxing --bare --no-setup` |
| opam switch | `fingerprintjs-compare` | OCaml 5.3.0, created |
| Dune | 3.24.2 | |
| Melange | `melange.7.0.1-53` | The 5.3-compatible 7.0.1 build |
| js_of_ocaml | 6.4.1 | `js_of_ocaml`, `js_of_ocaml-compiler`, `js_of_ocaml-ppx`, `js_of_ocaml-lwt` |
| Lwt | 6.1.2 | |

## Environment

```sh
eval "$(opam env --switch=fingerprintjs-compare --set-switch)"
```

Install, once the switch exists (do this from Wave 0, not from two agents at once):

```sh
opam install -y dune melange.7.0.1-53 js_of_ocaml js_of_ocaml-ppx js_of_ocaml-lwt
```

| Dart SDK | 3.13.0 | `/opt/homebrew/bin/dart`; `dart compile js` |
| Elm | 0.19.2 | `/usr/local/bin/elm` |
| Kotlin Toolchain | 0.11.1 | readable `js/app` lane at `~/.local/bin/kotlin` |
| Kotlin Gradle plugin | 2.4.10 | true production lane, same Kotlin sources |
| Gradle | 9.4.1 | downloaded to `experiments/fingerprintjs-kotlin/build/toolchains` unless `GRADLE_BIN` is set; supports the machine's Java 26 runtime |
| Coursier bin | | `$HOME/Library/Application Support/Coursier/bin` (not on default PATH) |
| scala-cli | 1.16.0 | Coursier; Scala 3.8.4; Scala.js 1.22.0 (hello compile warmed) |
| sbt runner | 2.0.6 | Coursier; prefer scala-cli for this experiment |

## PATH for Scala

```sh
export PATH="$HOME/Library/Application Support/Coursier/bin:$PATH"
```

## Kotlin lanes

The readable row uses Toolchain 0.11.1's `js/app` product. A `module.yaml` with
`product: js/app` emits modules under `build/tasks/_<module>_linkJs`; its
`-v release` argument is ignored.

The production row uses `gradle-production/` with Kotlin/JS Gradle plugin
2.4.10 and Gradle 9.4.1. `compileProductionExecutableKotlinJs` compiles the
same `src/` directory as whole-program ES modules targeting ES2015, with
production DCE and member-name minimization. `tools/build.mjs` downloads the
pinned Gradle distribution when `GRADLE_BIN` is not supplied.

## Oracle JS tools

Ports that need Rollup or Terser import them from
`experiments/fingerprintjs/node_modules`. Do not install a second copy.
Terser options are `experiments/fingerprintjs/terser.config.ts`
(`comments: false`, `safari10: true`).
