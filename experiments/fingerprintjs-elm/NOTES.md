# Elm 0.19.2 port notes

Elm for murmur, canonical `key:value|` join, confidence, the 42-key order, and
the math source. A counted JS host under `js/` owns every `navigator` /
`screen` / `matchMedia` / DOM / canvas / WebGL / audio probe, `mapWithBreaks`,
and the 25-export ESM facade.

**mostly JS.** `jsHost` (63 352 raw bytes of `js/` minus `facade.js`) is more
than 50% of the minified IIFE (57 980). That is the expected shape: Elm 0.19
cannot touch the browser except through ports, so the probes stay in JS.

## How to build

Elm 0.19.2 at `/usr/local/bin/elm`. Oracle Rollup / Terser from
`experiments/fingerprintjs/node_modules` (no second Terser).

```sh
node tools/build.mjs
```

That is:

```sh
elm make src/Fingerprint.elm --optimize --output=build/elm.js
elm make src/Runtime.elm --optimize --output=build/elm-runtime.js
```

then wrap the Elm IIFE as `build/elm.mjs`, Rollup `js/facade.js`, minify with
the oracle Terser (`comments: false`, `safari10: true`), prepend
`../fingerprintjs-compare/banner.txt`, and copy the oracle `fp.d.ts`.

Hash / canonical vectors (no browser):

```sh
node tools/test-hash.mjs
```

Compare-suite checks:

```sh
node ../fingerprintjs-compare/tools/check-exports.mjs --port elm
node ../fingerprintjs-compare/tools/check-ssr.mjs --port elm
node ../fingerprintjs-compare/tools/run-differential.mjs --port elm
```

## Elm / JS boundary

Ports or `Elm.Fingerprint.init` are the only boundary. No Elm kernel / native
modules.

| Port | Direction | Payload |
| --- | --- | --- |
| `request` | JS → Elm | `{ op, ... }` (see below) |
| `response` | Elm → JS | `{ result }` or `{ error }` |

`request.op` values:

| op | Elm work |
| --- | --- |
| `hash` | Murmur x64 128 from UTF-16 code units (`units`) plus `seed` |
| `canonical` | sort keys, escape `:|\`, join `key:value\|` |
| `hashComponents` | canonical join then hash (kept; the facade hashes via `canonical` + `hash`) |
| `confidence` | open-source score + Pro comment from platform + sniff flags |
| `math` | assemble the 23-key math fingerprint (polyfills in Elm, native samples from JS) |
| `sourceOrder` | the 42 built-in source names |

Elm 0.19 outgoing ports initialize with `_Process_sleep(0)`, so the first
(and every) response would miss the same `send()` turn. `tools/wrap-elm.mjs`
rewrites that init to `_Scheduler_succeed(null)` (the incoming-port task)
and rebinds the Elm IIFE from `this` to a module scope. Without the sleep
patch `hashComponents` / `murmurX64Hash128` cannot be synchronous, which the
public API and `fp.d.ts` require.

Unpaired UTF-16 surrogates cannot be walked as separate units by Elm
`String.foldl` (a high surrogate is always glued to the next unit). The JS
host therefore sends `charCodeAt` units; Elm still owns the UTF-8 encode
(U+FFFD replacement, matching `TextEncoder`) and the murmur lanes.

## Elm modules

| File | Owns |
| --- | --- |
| `src/Fingerprint.elm` | port worker, request dispatch |
| `src/Hashing.elm` | MurmurHash3 x64 128 (16-bit lanes) |
| `src/Utf8.elm` | TextEncoder-compatible UTF-8 |
| `src/Canonical.elm` | `key:value\|` join |
| `src/Confidence.elm` | score + comment template |
| `src/MathSource.elm` | math fingerprint + polyfills |
| `src/Sources.elm` | 42-key collection order |
| `src/Runtime.elm` | empty worker, size of the Elm runtime only |

## JS host files

| File | Role |
| --- | --- |
| `js/facade.js` | 25 named exports + `default` identity |
| `js/elm_bridge.js` | lazy `Elm.Fingerprint.init`, sync port RPC, UTF-16 units |
| `js/agent.js` | `load` / `hashComponents` / `componentsToDebugString` / `prepareForSources` / lazy `visitorId` |
| `js/async.js` | `mapWithBreaks` (16 ms `Date.now`, `MessageChannel`, `Promise.all`) |
| `js/entropy.js` | `loadSources` / `transformSource` / `loadSource` |
| `js/browser.js` | `isAndroid` / `isWebKit` / … / `getFullscreenElement` |
| `js/dom.js` | `withIframe`, selector helpers |
| `js/data.js` | `toInt` / `round` / `parseSimpleCssSelector` |
| `js/misc.js` | `errorToObject`, `isFunctionNative` |
| `js/sources/index.js` | 42-key `sources` object |
| `js/sources/simple.js` | navigator / screen / storage / plugins / UA-CH / architecture / Apple Pay |
| `js/sources/media.js` | `matchMedia` sources |
| `js/sources/math.js` | native `Math.*` samples for the Elm math source |
| `js/sources/screen_frame.js` | screen frame + watch |
| `js/sources/fonts.js` | iframe font probe |
| `js/sources/font_preferences.js` | natural-font measurements |
| `js/sources/dom_blockers.js` | content-blocker selectors |
| `js/sources/canvas.js` | canvas geometry / text |
| `js/sources/webgl.js` | WebGL basics + extensions |
| `js/sources/audio.js` | OfflineAudioContext + `audioBaseLatency` |

Nothing from `experiments/fingerprintjs/src` was copied into `js/`.

## Byte split

From `dist/attribution.json` after `elm make --optimize`:

| Bucket | bytes | What it is |
| --- | ---: | --- |
| `elmProgram` | 38033 | optimized `Fingerprint.elm` emit minus empty worker |
| `elmRuntime` | 60935 | optimized empty `Runtime.elm` worker |
| `jsHost` | 63352 | `js/**` except `facade.js` |
| `facade` | 1370 | `js/facade.js` |
| `fp.min.js` | 57980 | gated minified IIFE (includes MIT banner) |

`jsHost / fp.min.js` ≈ 109%. **mostly JS.**

| Artifact | raw | gzip-9 |
| --- | ---: | ---: |
| `dist/fp.esm.js` | 143839 | 36549 |
| `dist/fp.esm.min.js` | 57914 | 22958 |
| `dist/fp.min.js` | 57980 | 22992 |
| `dist/fp.d.ts` | 27932 | 8255 |

Size gates will fail. That is a result: Elm runtime + a full JS probe host is
larger than handwritten TypeScript.

## Parity

```
node ../fingerprintjs-compare/tools/run-differential.mjs --port elm --repetitions 1
chrome: 0 normalized difference(s)
firefox: 0 normalized difference(s)
safari: 0 normalized difference(s)

node ../fingerprintjs-compare/tools/check-exports.mjs --port elm
ESM namespace matches the v5.2.0 contract

node ../fingerprintjs-compare/tools/check-ssr.mjs --port elm
ESM import is SSR-safe
```

`fp.d.ts` is the oracle file (SHA-256
`d20e2e6b3f60afb7679c1f87dd62800b7a7ce7155e86efbf6206190a1c185731`).
`LoadOptions.monitoring` is typed and defaulted and is a no-op (no XHR / fetch
/ beacon).

## Compiler-aware production minification (2026-08-15)

The production variant follows Elm's documented minifier assumptions (`F2`–`F9`, `A2`–`A9`, pure getters, no kept function arity, and the Elm-safe unsafe comparison/optimization flags). Those assumptions are applied to `build/elm.mjs` only. The handwritten JS host is bundled and minified with ordinary JavaScript-safe settings.

| Variant | IIFE min | gzip-9 | brotli-11 |
| --- | ---: | ---: | ---: |
| `dist/standard` | 57,980 | 22,992 | 20,488 |
| `dist/production` | 53,949 | 21,964 | 19,616 |

The row remains labeled mostly JS; production minification does not change the ownership split.
