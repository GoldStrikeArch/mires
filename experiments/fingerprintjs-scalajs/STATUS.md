> **Historical record.** This file describes the archived port, not a fresh qualification. Current restoration/build results and limitations are in `../../docs/COMPARATORS.md`.

# Status

Scala.js 1.22.0 / Scala 3.8.4 port of FingerprintJS 5.2.0. Builds, is SSR-safe, and matches the oracle on Chrome, Firefox, and Safari (1-repetition differential).

## Works

- Artifacts: `dist/fp.esm.js`, `dist/fp.esm.min.js`, `dist/fp.min.js`, `dist/fp.d.ts`
- Public exports, default-export identity, 42-source registry, murmur x64, lazy visitor id
- Custom `loadSources` / `transformSource` / two-stage sources
- `monitoring` no-op
- Differential: Chrome / Firefox / Safari 0 diffs
- SSR import check

## Does not

- Size gates (expected; `fastLink` ESM is ~995 KiB, minified IIFE gzip-9 is 91 590 bytes)
- CJS / UMD (optional for this port)
- Shipped emit is `fastLink`, not `fullLink` / `--js-mode release` (release is 608 539 raw bytes; see NOTES)

## Build

```sh
export PATH="$HOME/Library/Application Support/Coursier/bin:$PATH"
cd experiments/fingerprintjs-scalajs
node tools/build.mjs
```

scala-cli 1.16.0 comes from Coursier. Rollup/Terser come from `../fingerprintjs/node_modules`.

## Production-profile follow-up (2026-08-15)

The build now publishes two complete variants:

- `dist/fast/`: `fastLinkJS`, retained as the readable-first row;
- `dist/full/`: `fullLinkJS` / release, used as the production-default row.

Root `dist/fp.*` remains an alias of `fast`. `scalajs-full` passes exports, SSR import, and a fresh one-repetition Chrome / Firefox / Safari differential with zero normalized differences. Its minified IIFE is 228,513 B, gzip-9 67,338 B, and Brotli-11 56,734 B.
