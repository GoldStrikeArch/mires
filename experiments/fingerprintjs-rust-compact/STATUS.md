# Compact Rust FingerprintJS 5.2.0

Unpublished research port. Package and crate name: `fingerprintjs-rust-compact-experiment`. Reported algorithm version: **5.2.0**.

This is the copy-then-mutate compact experiment. It is not the idiomatic rust verdict and not a shipping replacement. 100% safe Rust (`unsafe_code = "deny"`), `no_std`, no `js!`, no product `.js`. Boundary `mapWithBreaks` only.

## Build

Pinned toolchain is `nightly-2026-07-27` in `rust-toolchain.toml`. Oracle Rollup/Terser come from `experiments/fingerprintjs/node_modules`.

```sh
cd experiments/fingerprintjs-rust-compact
npm run build
# same as:
# node tools/build.mjs --optimizer on --rust-opt-level z --mir-inline on --scheduler boundary
```

Default `dist/` is that single build (no off/fiber matrix).

Shipped files:

- `dist/fp.esm.js`
- `dist/fp.esm.min.js`
- `dist/fp.min.js`
- `dist/fp.d.ts` (oracle declaration; SHA-256 `d20e2e6b3f60afb7679c1f87dd62800b7a7ce7155e86efbf6206190a1c185731`)

CJS/UMD wrappers are also written.

## Parity

```
node experiments/fingerprintjs-compare/tools/run-differential.mjs --port rust-compact
node experiments/fingerprintjs-compare/tools/check-ssr.mjs --port rust-compact
```

| Check | Result |
| --- | --- |
| Chrome / Firefox / Safari differential | **Pass**, 0 normalized differences (two alternating captures) |
| SSR-safe ESM import | **Pass** |
| 25 exports, default export identity, 42 sources in order | **Pass** (`check-package.mjs`) |
| `fp.d.ts` checksum | **Pass** |
| Size gates vs upstream 16 KB gzip IIFE | **Fail** (expected) |

Native `cargo +nightly-2026-07-27 test --lib`: 19 unit tests pass. Product logic is the JS artifact.

## Size vs idiomatic rust `dist/boundary`

Same gzip-9 / brotli-11 method as `tools/common.mjs`. Compact numbers are `bench/results/bundle-sizes.json`.

| Artifact | Idiomatic `dist/boundary` | Compact `dist/` | Change |
| --- | ---: | ---: | ---: |
| Minified ESM raw | 332,381 | 192,850 | −42.0% |
| Minified ESM gzip-9 | 73,939 | 53,645 | −27.4% |
| Minified ESM brotli-11 | — | 41,564 | |
| Minified IIFE raw | 332,381-class / VERDICT 340,309 | 192,830 | about −43% vs VERDICT IIFE |
| Minified IIFE gzip-9 | VERDICT 76,175 | 53,594 | −29.6% vs VERDICT |
| Minified IIFE brotli-11 | VERDICT 60,833 | 41,549 | −31.7% vs VERDICT |
| Raw compiler program | VERDICT on-boundary ~947,810 | 614,026 | |

Same-compiler-flag neighbors of the frozen port (not the default `dist/boundary`):

| Idiomatic artifact | ESM min raw | Compact | Change |
| --- | ---: | ---: | ---: |
| `dist/on/fp.esm.min.js` (optimizer on) | 277,937 | 192,850 | −30.6% |
| `dist/on-mir-inline/fp.esm.min.js` | 293,619 | 192,850 | −34.3% |
| `dist/on-rust-z/fp.esm.min.js` | 302,832 | 192,850 | −36.3% |

Upstream gated IIFE is 36,746 / 16,178 gzip / 14,239 brotli. Compact IIFE is still 5.25× raw and 3.31× gzip. The size gate fails. The compact port is how much smaller 100% Rust can get without leaving Rust or patching the compiler.

## After the `slice_error_fail_rt` sysroot stub

Rebuilt 2026-08-14 against the extended panic-without-formatting patch. Three-browser differential still 0 diffs.

| Artifact | This file, above | After stub |
| --- | ---: | ---: |
| Minified ESM raw | 192,850 | **159,764** |
| Minified IIFE gzip-9 | 53,594 | **43,910** |
| Raw compiler program | 614,026 | **521,642** |

`fmt::write`, `escape_debug`, `flt2dec`, and `pad_integral` are gone from `dist/raw/program.js`. A `unicode_data::white_space` map remains from `is_whitespace`.

## What this experiment is

Ugly but safe Rust. Host Promises instead of async. Unboxed murmur. Components stay `JsValue`. See [NOTES.md](NOTES.md) for the deletion list and leftover `core` items.
