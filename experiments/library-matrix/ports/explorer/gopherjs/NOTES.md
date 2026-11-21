# Explorer: GopherJS port

Hand-written Go composition driver for the Offline Data Explorer application (`libraries/explorer/CONTRACT.md`), compiled with GopherJS and linked by `languages/composite/build.mjs` with the five GopherJS component ports (`idb`, `minisearch`, `marked`, `fflate`, `uplot`).

## Layout

| File | What it is |
| --- | --- |
| `src/main.go` | Step-function driver (`run`, `persist`, `stored`, `hits`, `list`, `object`), exporting `run` on `js.Module.exports` |
| `components.lock.json` | Content-addressed lock of the five GopherJS component source inventories, compiled artifacts, toolchain, and composite linker |

## Boundary and composite loop

The composite linker (`languages/composite/build.mjs`) calls `run([input, history])` in a loop, invoking each requested GopherJS component (`idb`, `minisearch`, `marked`, `fflate`, `uplot`), awaiting thenable results, appending results to `history`, and returning `argument` when the driver returns `[null, argument]`.

When a component returns a typed array (`ArrayBuffer.isView(value)`), the linker converts it with `Array.from(value)` before pushing to `history`. Thus `fflate`'s `strToU8` and `gzipSync` `Uint8Array` results reach Go as plain JS arrays (`history.Index(3)` and `history.Index(4)`), so step 4 passes `history.Index(3)` straight as `data.bytes`, and the final snapshot uses `history.Index(3).Length()` and returns `history.Index(4)`.

## Component invocation order

1. **`idb` (`history.Length() == 0`)**: serializes each `input.records` document via host `JSON.stringify` into `{ id, group: "documents", value }` and builds the three-step `idb` script (`openDB` creating store `rows` with `keyPath: "id"` and index `byGroup` on `group`; `readwrite` batch of `put`s with `done: true`; `readonly` `openCursor` walk with `rest: [["continue"]]`; every step/request setting `catch: false`).
2. **`minisearch` (`1`)**: walks `history[0].log` for `"cursor"` entries, extracts each entry's `value` string, parses each via host `JSON.parse`, and requests `addAll` + `search` (`input.query`) with `fields: ["title", "body"]` and `storeFields: ["body"]`.
3. **`marked` (`2`)**: reads `history[1].results[1].hits`, selects `hits[0].body` (or `""` when empty), and requests `{ op: "parse", markdown }`.
4. **`fflate` `strToU8` (`3`)**: joins `stored(history)` with `strings.Join(..., "\n")` and requests `{ op: "strToU8", text }`.
5. **`fflate` `gzipSync` (`4`)**: passes `history.Index(3)` as `data.bytes` with `{ level: 6, mtime: 0 }`.
6. **`uplot` (`5`)**: forwards `input.Get("chart")` directly to the GopherJS `uplot` port.
7. **Snapshot (`default`)**: returns `[null, [ids, history[2], history[4], history[3].Length(), history[5]]]`.

## GopherJS-specific details

- Uses `type object = map[string]interface{}` (which GopherJS externalizes to plain JS objects) and `list(...)` backed by `js.Global.Get("Array").New()`.
- Uses host `JSON.stringify` and `JSON.parse` (via `js.Global.Get("JSON")`) so document key insertion order is preserved in the gzip archive bytes.
- **Rollup tree shaking disabled**: per `languages/composite/build.mjs` and `libraries/explorer/CONTRACT.md`, GopherJS composite linking runs Rollup with `treeshake: false` so labelled breaks in GopherJS's generated Go control flow are preserved across the already-optimized modules.

## Deviations, module state, and accommodations

- **Deviations from `reference.mjs`**: none; typed-array conversion (`Array.from`) of `fflate` results happens in the shared composite linker loop.
- **Module state**: none.
- **Performance accommodations**: none.
