> **Historical record.** This file describes the archived port, not a fresh qualification. Current restoration/build results and limitations are in `../../docs/COMPARATORS.md`.

# Status

ReScript 12.3.0 port of FingerprintJS 5.2.0. Builds, is SSR-safe, and matches the oracle on Chrome, Firefox, and Safari (1-repetition differential).

## Works

- Artifacts: `dist/fp.esm.js`, `dist/fp.esm.min.js`, `dist/fp.min.js`, `dist/fp.d.ts`
- Public exports, default-export identity, 42-source registry, murmur x64, lazy visitor id
- Custom `loadSources` / `transformSource` / two-stage sources
- `monitoring` no-op
- Differential: Chrome / Firefox / Safari 0 diffs
- SSR import check

## Does not

- Size gates (expected; bundle is larger than the TypeScript oracle)
- CJS / UMD (optional for this port)

## Build

```sh
cd experiments/fingerprintjs-rescript
npm install   # rescript@12.3.0 only; Rollup/Terser come from ../fingerprintjs/node_modules
node tools/build.mjs
```
