> **Historical comparison archive.** The matrices in this directory are retained historical results. Use `npm run build:comparators`, `npm run check:comparators`, and `node experiments/tools/measure-comparators.mjs` for the restored checkout. See `../../docs/COMPARATORS.md`.

# FingerprintJS compiler comparison

Shared contract, production/readable profile registry, measurement harness, and report for the FingerprintJS 5.2.0 ports. The ports themselves live in sibling directories. This package points the existing Rust contract, browser differential, and benchmark harnesses at each emitted artifact.

See [CONTRACT.md](CONTRACT.md) for the public program, allowed techniques, profile rules, and runtime-first decision order. See [toolchains.md](toolchains.md) for pinned compiler versions. The historical analysis and the production-profile correction are in [REPORT.md](REPORT.md).

```sh
# Node-only namespace and SSR checks
node tools/check-exports.mjs --port dart-o2
node tools/check-ssr.mjs --port dart-o2

# Three-browser parity against the TypeScript oracle
node tools/run-differential.mjs --port dart-o2

# Size matrix: all rows and production-only view
node tools/measure-matrix.mjs --output matrix.json
node tools/summarize.mjs --input matrix.json --output matrix.md
node tools/summarize.mjs --input matrix.json --profile production --output matrix.production.md

# Runtime-first production matrix
node tools/run-runtime-matrix.mjs
node tools/summarize-runtime.mjs

# Fast local smoke and isolated hot-kernel diagnostic
node tools/run-runtime-matrix.mjs --smoke true
node tools/run-kernel-matrix.mjs
```

Explicit production variant IDs are `dart-o2`, `scalajs-full`, `elm-production`, and `kotlin-production`. Their readable-first neighbors remain `dart`, `scalajs`, `elm`, and `kotlin`. `kotlin-proxy` preserves the old tree-shaken `js/app` packaging result but is excluded from production rankings. Root `dist/fp.*` files remain readable aliases for compatibility.

The Rust experiment at `../fingerprintjs-rust` is frozen. Do not edit it from the comparison harness.

## Strict FingerprintJS feature-parity audit

The original differential proves the frozen benchmark contract. That contract deliberately keeps `LoadOptions.monitoring` as a typed no-op, so it is not by itself proof of literal upstream behavior.

The stricter audit separates:

- the exact 42-source agent/component/custom-source/hash contract;
- upstream side effects such as installation monitoring and debug logs;
- direct utility and rejection behavior;
- observable function metadata;
- IIFE namespace shape and oracle artifact-format coverage.

Run:

```sh
npm run parity:feature:static
npm run parity:feature:browser
npm run parity:feature:summary
```

The generated reports are `feature-parity-static.json`, `feature-parity.json`, and `feature-parity.md`. The root-level interpretation and fix order are in `../../FINGERPRINTJS-FEATURE-PARITY-AUDIT.md`.
