# FingerprintJS ReScript port notes

## Toolchain

- ReScript 12.3.0 (`npx --yes rescript@12.3.0`, also pinned in `devDependencies`)
- Node v24.5.0
- Oracle Rollup / Terser from `../fingerprintjs/node_modules` (no second Terser)

`rescript.json` emits ESM (`package-specs.module: esmodule`, suffix `.res.mjs`) into `lib/es6`. `tools/build.mjs` compiles with ReScript, bundles `lib/es6/src/Index.res.mjs` with the oracle Rollup + `@rollup/plugin-node-resolve`, minifies with the oracle Terser (`comments: false`, `safari10: true`), prepends the FingerprintJS MIT banner, and copies the oracle `fp.d.ts`.

## `@rescript/runtime`

The compiler emit can import `@rescript/runtime` (e.g. `Primitive_option` if option helpers are used). The current product graph does not keep any of those imports after Rollup tree-shaking. Shipped `dist/fp.esm.js` has no `@rescript/runtime` import and no inlined runtime helpers. Runtime bytes for this port are **0**.

## `%raw`

Used only where a typed external cannot express the host operation:

- JS operators: `in`, `instanceof`, `typeof`, `=== undefined` / `null`, `||`, `??`, `!!`, `new C(...)` with a dynamic constructor
- Dynamic apply/`this` for getters (`makeGetterUsingThis`) and `try/finally` around async iframe work
- A few string/index helpers and `arguments[0]` for `getDomBlockers` so `function.length` stays 0

No product `.js` sources.

## Public program

- 25 named exports in `frozen.json` order
- `default` is `{ load, hashComponents, componentsToDebugString }` with identity equality
- 42 sources in collection order
- Murmur x64 matches the oracle vectors
- Lazy `visitorId` getter/setter (enumerable, named `get visitorId` / `set visitorId`)
- Scheduler is upstream `mapWithBreaks` only (16 ms `Date.now()` budget, `MessageChannel` yield, `Promise.all`)
- `LoadOptions.monitoring` is typed and defaulted but a no-op (no XHR/fetch/beacon)
- SSR-safe: browser globals are only read inside functions
- Algorithm version `5.2.0`
- `dist/fp.d.ts` is the oracle file (SHA-256 `d20e2e6b3f60afb7679c1f87dd62800b7a7ce7155e86efbf6206190a1c185731`)

## Parity

`node ../fingerprintjs-compare/tools/run-differential.mjs --port rescript --repetitions 1` and `check-ssr.mjs --port rescript`:

- Chrome: 0 normalized differences
- Firefox: 0 normalized differences
- Safari: 0 normalized differences
- ESM import is SSR-safe

## Size

Size gates will fail. That is expected. Unminified ESM is larger than the handwritten TypeScript oracle because ReScript emit is more verbose (explicit host helpers, recursive loops). Minified IIFE gzip is the gated number and is not competitive with the oracle.
