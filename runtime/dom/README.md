# `runtime/dom`, the vendored client runtime

A single self-contained ESM file that provides the **dom-expressions client
ABI** to code produced by the Rust to JS emitter.

`dist/mires-dom.js` is committed. It is a build artifact, but it is checked in
deliberately: the emitter's output is meaningless without it, and a reviewer
should be able to see the exact bytes the generated code will run against
without needing a JS toolchain.

## What it is

`src/index.js` re-exports, from `solid-js/web`, exactly the 48 names in
[`contract/fixtures/abi.json`](../../contract/fixtures/abi.json), the complete
export surface of `dom-expressions/src/client.js`, plus the two reactive names
in [`reactive-additions.mjs`](reactive-additions.mjs), `createSignal` and
`runWithOwner`, from `solid-js`. That is 50 exports. `build.mjs` bundles the entry point, with solid inlined, into one file
with no remaining imports.

### Why anything is added to the ABI

`abi.json` is extracted by importing upstream's `client.js` and reflecting over
it, so it can only ever describe what upstream exports. The emitter needs two
names upstream does not have: `createSignal`, for a signal declared in an island,
and `runWithOwner`, which the async executor uses to re-enter a captured owner
after an await. `client.js` re-exports six names from the rxcore seam (`effect`,
`memo`, `untrack`, `getOwner`, `createComponent`, `mergeProps`); creating state
is not one of them, and neither is `getOwner`'s counterpart.

Neither can come from a second copy of solid loaded next to this one: a signal
created in another reactive graph never notifies the effects in this one, so
hydrated state would silently never update. They have to be exported by this
artifact, from the same inlined copy of solid's core. `reactive-additions.mjs` is
where such a name is pinned, `gen-index.mjs` writes it into the entry point, and
`check-exports.mjs` expects the union of the two lists. `check-exports.mjs` also
runs the signal and effect against each other, so "one graph" is asserted, not
assumed.

Adding `createSignal` cost 9 gzipped bytes, which is the other half of the same
evidence: nothing new was pulled in, only a name exported.

### Why solid-js/web and not dom-expressions

`dom-expressions` is not a runnable package. It ships `src/client.js` as raw ESM
importing from the bare specifier `rxcore`, the reactivity seam each host
framework fills in, and it has no `exports` map and no `type: "module"`.

`solid-js/web`'s browser build **is** that same `client.js` with `rxcore` already
bound to solid's reactive core. Bundling it yields the ABI and its reactivity in
one artifact.

This settles a question the plan flagged as open. The plan anticipated that some
ABI names might not be re-exportable from solid's public surface and would need
aliasing or another source. **None do.** All 48 names are present in
`solid-js/web`'s browser build, including all 24 the emitter is specified to
call. `contract/fixtures/abi.json` records this under `solidWebParity`, and
`gen-index.mjs` refuses to generate an entry point if that ever stops being true.
The additions are the separate question of names the ABI does not contain at all.

Equally, the **rxcore seam needs no wiring on our side**, it is already
satisfied inside the artifact.

### The browser/server condition trap

`solid-js/web` resolves differently per export condition. Under node it resolves
to `dist/server.js`, the SSR build, where most DOM functions are bound to a
`notSup` throwing stub. `build.mjs` sets `platform: "browser"` and
`conditions: ["browser", "import"]` so the real client build
(`dist/web.js`) is bundled. This is easy to get wrong and produces an artifact
that looks correct, right export names, right count, but throws on use.
`check-exports.mjs` guards the names; the condition settings guard the bodies.

## Layout

| path | what |
|---|---|
| `src/index.js` | **generated** re-export list, do not edit |
| `reactive-additions.mjs` | the pinned names the emitter needs that the ABI lacks |
| `gen-index.mjs` | regenerates `src/index.js` from the ABI fixture and the additions |
| `build.mjs` | the bundler invocation, plus size measurement |
| `check-exports.mjs` | asserts the built bundle's exports equal that surface exactly |
| `dist/mires-dom.js` | the artifact |
| `dist/mires-dom.js.map` | source map |
| `dist/mires-dom.js.maxbytes` | measured gzipped size baseline |

## Rebuilding

```sh
yarn install          # exact versions from yarn.lock
node gen-index.mjs    # regenerate src/index.js from the ABI fixture
node build.mjs        # produce dist/
node check-exports.mjs
```

`yarn build` and `yarn check` are the same last two steps.

## Sizes

| | bytes |
|---|---|
| raw | 23,971 |
| gzip | 9,106 |

`dist/mires-dom.js.maxbytes` holds the gzipped number. It is a **measured
baseline, not a budget**: `build.mjs` prints the delta against it on every
rebuild so unexplained growth is visible in review. Update it deliberately when
the artifact legitimately grows.

## Reproducibility expectation

The build is deterministic. Two runs from the same lockfile produce a
byte-identical `dist/mires-dom.js`; this was verified by rebuilding and
comparing sha256. Anything that changes those bytes should be one of:

1. a change to `src/index.js`, which can only come from a change to
   `contract/fixtures/abi.json` (i.e. an upstream ABI change) or to
   `reactive-additions.mjs`, or
2. a deliberate version bump in `package.json` / `yarn.lock`, or
3. a change to the settings in `build.mjs`.

If the bytes move for any other reason, treat it as a supply-chain question
rather than noise.

`solid-js` is pinned to `1.9.14`, the same version
[`contract/upstream.lock`](../../contract/upstream.lock) pins for the contract
extraction, so the runtime and the contract describe the same code.

## Bundler choice

`esbuild`, not `tsup`. The precedent in this repo,
`crates/topcoat-runtime/browser`, uses tsup, and every setting here mirrors its
`tsup.config.ts` exactly: `format: esm`, bundle everything, `platform: browser`,
`target: es2022`, `minify`, `sourcemap`.

tsup's value is TypeScript entry handling and `.d.ts` emission. This package has
a single hand-written `.js` entry and no types to emit, so tsup would contribute
only a wrapper around the esbuild call it makes anyway. Using esbuild directly
removes a layer between the pin and the artifact. If this package ever grows a
TypeScript surface, switching to tsup is the right move and the settings
transfer unchanged.

## Build settings worth knowing

- `packages: "bundle"` + `external: []`, nothing is left as an import. The
  build fails if any bare import survives into the output.
- `define: { "process.env.NODE_ENV": '"production"' }`, solid branches on this;
  without it the dev build's warnings and instrumentation are retained.
- `legalComments: "none"`, licence text is excluded from the artifact. solid is
  MIT; the licence obligation is met by the dependency declaration and lockfile,
  not by inlining a banner into every page's JS payload.
