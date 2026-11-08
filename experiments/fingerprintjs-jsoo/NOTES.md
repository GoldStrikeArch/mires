# js_of_ocaml port notes

Whole-program `js_of_ocaml` 6.4.1 emit of FingerprintJS 5.2.0. The runtime tax is the measurement; stdlib was not stripped.

## Compilation

- Mode: whole-program (`dune` `(compilation_mode whole_program)`), not separate compilation.
- Effects: `--effects=disabled` (classic CPS-free runtime). Lwt 6.1.2 is used without effect handlers.
- Target: `--target-env=browser --no-extern-fs` so the emit does not construct Node `fs` or touch `window`/`document`/`self` at import time.
- Other flags: `--opt=3`, `use-js-string=true` (JSOO default on this switch).
- Product link: `js_of_ocaml` + `lwt` only. `js_of_ocaml-lwt` is installed on the switch but is not linked; `MessageChannel` yield and `setTimeout` are bound through `Js.Unsafe`.
- Unminified ESM is a second `--pretty` compile of the same bytecode. Minified ESM / IIFE are Terser (`comments: false`, `safari10: true`) from the oracle `node_modules`, wrapping the compact whole-program emit.

Build:

```sh
eval "$(opam env --switch=fingerprintjs-compare --set-switch)"
node tools/build.mjs
```

## Exact opam versions (`opam list --installed`)

Recorded 2026-08-14 on switch `fingerprintjs-compare` (OCaml 5.3.0):

| Package | Version |
| --- | --- |
| ocaml / ocaml-base-compiler | 5.3.0 |
| dune | 3.24.2 |
| js_of_ocaml | 6.4.1 |
| js_of_ocaml-compiler | 6.4.1 |
| js_of_ocaml-ppx | 6.4.1 |
| js_of_ocaml-lwt | 6.4.1 (installed, not linked) |
| lwt | 6.1.2 |
| ocamlfind | 1.9.8 |
| ppxlib | 0.38.0 |

## Runtime vs program bytes

Sizes are raw bytes of the JSOO emit *before* the ESM/IIFE facade. Gzip-9 / brotli-11 match the rust `tools/common.mjs` method.

| Artifact | raw | gzip-9 | brotli-11 |
| --- | ---: | ---: | ---: |
| Whole-program emit (`dist/raw/program.js`) | 138894 | 48051 | 41852 |
| `--no-runtime` program only (`dist/raw/program-only.js`) | 102409 | 33412 | 28732 |
| DCE'd runtime included (whole − no-runtime) | **36485** | — | — |
| Full unused runtime (`js_of_ocaml print-standard-runtime`) | 469772 | 94142 | 77030 |

The interesting number is the **36 485-byte DCE'd JSOO runtime** that remains after whole-program inlining, versus rustc-codegen-js's extracted shim. The unused full runtime is ~470 KiB; DCE keeps about 8% of it.

`--no-runtime` is missing the usual `+sys.js` / `+jslib.js` / `+io.js` primitives (JSOO warns and emits dummy stubs). It is a size attribution, not a runnable program.

## Shipped artifacts

| File | raw | gzip-9 | brotli-11 |
| --- | ---: | ---: | ---: |
| `dist/fp.esm.js` (pretty + facade) | 358357 | 76944 | 63327 |
| `dist/fp.esm.min.js` | 136383 | 48301 | 41815 |
| `dist/fp.min.js` (IIFE, gated number) | 136140 | 48307 | 41870 |
| `dist/fp.d.ts` | oracle copy, SHA-256 `d20e2e6b3f60afb7679c1f87dd62800b7a7ce7155e86efbf6206190a1c185731` |

Size gates will fail. That is the result: idiomatic OCaml + stdlib + JSOO runtime is larger than handwritten TypeScript.

## Parity

```
node ../fingerprintjs-compare/tools/run-differential.mjs --port jsoo
chrome: 0 normalized difference(s)
firefox: 0 normalized difference(s)
safari: 0 normalized difference(s)

node ../fingerprintjs-compare/tools/check-ssr.mjs --port jsoo
ESM import is SSR-safe
```
