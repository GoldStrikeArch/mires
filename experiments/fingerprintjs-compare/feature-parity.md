# FingerprintJS feature-parity audit

Generated 2026-08-15T18:23:11.641Z. Oracle commit `e196578ba35362fdf15647e013d66ac28b3c9fb5`.

“Core fingerprinting” is the exact 42-source agent/custom-source/hash contract in all three local browsers. “Upstream effects” additionally includes installation monitoring, debug logs, direct utility calls, iframe error propagation, and cleanup. The latter is intentionally stricter than the historical comparison contract.

| Implementation | Core fingerprinting | Upstream effects | Function metadata | API keys + `.d.ts` | IIFE shape | Oracle formats | Strict ESM | Artifact package |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Idiomatic Rust | **PASS** | **PASS** | FAIL | **PASS** | **PASS** | **PASS** | FAIL | FAIL |
| ReScript | **PASS** | **PASS** | FAIL | **PASS** | **PASS** | FAIL | FAIL | FAIL |
| Melange 7.0.1 | **PASS** | **PASS** | **PASS** | **PASS** | **PASS** | FAIL | **PASS** | FAIL |
| js_of_ocaml | **PASS** | **PASS** | **PASS** | **PASS** | FAIL | FAIL | **PASS** | FAIL |
| Compact Rust | **PASS** | **PASS** | FAIL | **PASS** | **PASS** | **PASS** | FAIL | FAIL |
| Hybrid Rust | **PASS** | **PASS** | FAIL | **PASS** | **PASS** | **PASS** | FAIL | FAIL |
| Dart2js -O2 (production) | **PASS** | **PASS** | FAIL | **PASS** | FAIL | FAIL | FAIL | FAIL |
| Scala.js fullLink (production) | **PASS** | **PASS** | FAIL | **PASS** | **PASS** | FAIL | FAIL | FAIL |
| Elm compiler-aware minifier | **PASS** | **PASS** | FAIL | **PASS** | **PASS** | FAIL | FAIL | FAIL |
| Kotlin/JS Gradle production | **PASS** | **PASS** | FAIL | **PASS** | FAIL | FAIL | FAIL | FAIL |

## Upstream behavioral and side-effect gaps

## Observable function-metadata gaps

- **Idiomatic Rust:** agent.get differ in observable name and/or arity. Exact values are in `feature-parity.json`.
- **ReScript:** 22 exported function(s); agent.get differ in observable name and/or arity. Exact values are in `feature-parity.json`.
- **Compact Rust:** agent.get and audio cleanup function differ in observable name and/or arity. Exact values are in `feature-parity.json`.
- **Hybrid Rust:** agent.get differ in observable name and/or arity. Exact values are in `feature-parity.json`.
- **Dart2js -O2 (production):** 4 exported function(s); agent.get differ in observable name and/or arity. Exact values are in `feature-parity.json`.
- **Scala.js fullLink (production):** 23 exported function(s); agent.get differ in observable name and/or arity. Exact values are in `feature-parity.json`.
- **Elm compiler-aware minifier:** 1 exported function(s) differ in observable name and/or arity. Exact values are in `feature-parity.json`.
- **Kotlin/JS Gradle production:** 23 exported function(s); agent.get differ in observable name and/or arity. Exact values are in `feature-parity.json`.

## IIFE namespace-shape gaps

- **js_of_ocaml:** minified IIFE export/default/source property order or default identities differ from the oracle.
- **Dart2js -O2 (production):** minified IIFE export/default/source property order or default identities differ from the oracle.
- **Kotlin/JS Gradle production:** minified IIFE export/default/source property order or default identities differ from the oracle.

## Missing oracle distribution formats

- **ReScript:** `fp.cjs.js`, `fp.js`, `fp.umd.js`, `fp.umd.min.js`.
- **Melange 7.0.1:** `fp.cjs.js`, `fp.js`, `fp.umd.js`, `fp.umd.min.js`.
- **js_of_ocaml:** `fp.cjs.js`, `fp.js`, `fp.umd.js`, `fp.umd.min.js`.
- **Dart2js -O2 (production):** `fp.cjs.js`, `fp.js`, `fp.umd.js`, `fp.umd.min.js`.
- **Scala.js fullLink (production):** `fp.cjs.js`, `fp.js`, `fp.umd.js`, `fp.umd.min.js`.
- **Elm compiler-aware minifier:** `fp.cjs.js`, `fp.js`, `fp.umd.js`, `fp.umd.min.js`.
- **Kotlin/JS Gradle production:** `fp.cjs.js`, `fp.js`, `fp.umd.js`, `fp.umd.min.js`.

