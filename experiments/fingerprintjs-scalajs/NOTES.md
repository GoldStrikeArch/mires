# Scala.js port notes

Idiomatic Scala 3.8 + Scala.js 1.22.0 emit of FingerprintJS 5.2.0. Readable output is `fastLink` (`--js-mode fast`). The gated IIFE is Terser over that same readable emit, matching the other ports.

## Compilation

- scala-cli 1.16.0, Scala 3.8.4, Scala.js 1.22.0
- `//> using jsModuleKind es` and `@JSExportTopLevel` for the 25 frozen names
- `org.scala-js::scalajs-dom::2.8.1` is on the compile classpath; host probes that FingerprintJS reads as untyped / vendor fields go through `js.Dynamic` so import stays SSR-safe
- Unminified ESM is `fastLink`. Minified ESM / IIFE are oracle Terser (`comments: false`, `safari10: true`) from `experiments/fingerprintjs/node_modules`
- No product `.js` sources

```sh
export PATH="$HOME/Library/Application Support/Coursier/bin:$PATH"
node tools/build.mjs
```

## Debug vs release

| Artifact | raw | gzip-9 | brotli-11 |
| --- | ---: | ---: | ---: |
| scala-cli `fastLink` (`target/fp.fast.js`) | 993996 | 141604 | 104083 |
| scala-cli `fullLink` / `--js-mode release` | 608539 | 106045 | 82708 |
| Empty `@JSExportTopLevel` `fullLink` (runtime floor) | 41969 | 7907 | 6904 |

`fullLink` is recorded as a footnote only. Shipped `fp.esm.js` is the readable `fastLink` emit plus the MIT banner.

## Runtime leftovers after DCE

Scala.js has no JSOO-style `--no-runtime`. The leftover is estimated by compiling a one-export empty module with `fullLink`:

| | raw bytes |
| --- | ---: |
| Full-program `fullLink` | 608539 |
| Empty-export `fullLink` (DCE'd Scala.js runtime floor) | **41969** |
| Program above that floor | 566570 |

The 42 KB floor is the interesting tax: `core` / `javalib` leftovers that survive DCE even when the program is a single `() => 1` export. The FingerprintJS program then adds ~567 KB of `fullLink` Scala before Terser.

## Shipped artifacts

| File | raw | gzip-9 | brotli-11 |
| --- | ---: | ---: | ---: |
| `dist/fp.esm.js` (fastLink + banner) | 995265 | 142354 | 104700 |
| `dist/fp.esm.min.js` | 485633 | 91340 | 73469 |
| `dist/fp.min.js` (IIFE, gated number) | 485126 | 91590 | 73517 |
| `dist/fp.d.ts` | oracle copy, SHA-256 `d20e2e6b3f60afb7679c1f87dd62800b7a7ce7155e86efbf6206190a1c185731` |

Size gates will fail. That is the result: idiomatic Scala 3 + Scala.js stdlib/runtime is larger than handwritten TypeScript.

## Parity

```
node ../fingerprintjs-compare/tools/run-differential.mjs --port scalajs --repetitions 1
chrome: 0 normalized difference(s)
firefox: 0 normalized difference(s)
safari: 0 normalized difference(s)

node ../fingerprintjs-compare/tools/check-ssr.mjs --port scalajs
ESM import is SSR-safe
```

## Explicit comparison variants (2026-08-15)

`tools/build.mjs` now packages both linker modes as complete comparison artifacts instead of keeping `fullLink` as a notes-only raw file.

| Variant | Linker | IIFE min | gzip-9 | brotli-11 |
| --- | --- | ---: | ---: | ---: |
| `dist/fast` | `fastLinkJS` | 485,126 | 91,364 | 73,517 |
| `dist/full` | `fullLinkJS` / release | 228,513 | 67,338 | 56,734 |

The production correction materially improves size, but the remaining program is still dominated by Scala collection, closure, exception, and class semantics rather than the approximately 42 KB empty-export runtime floor.
