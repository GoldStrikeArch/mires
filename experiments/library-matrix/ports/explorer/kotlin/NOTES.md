# Explorer: Kotlin/JS port

Hand-written Kotlin/JS composition driver for the Offline Data Explorer application (`libraries/explorer/CONTRACT.md`), linked by `languages/composite/build.mjs` with the five Kotlin/JS component ports (`idb`, `minisearch`, `marked`, `fflate`, `uplot`).

## Layout

| File | What it is |
| --- | --- |
| `src/main.kt` | Step-function driver (`persist`, `stored`, `command`, and `@JsExport fun run(packet: dynamic): dynamic`) |
| `components.lock.json` | Content-addressed lock of the five Kotlin/JS component source inventories, compiled artifacts, toolchain, and composite linker |

## Boundary and composite loop

The composite linker (`languages/composite/build.mjs`) calls `run([input, history])` in a loop, invoking each requested Kotlin/JS component (`idb`, `minisearch`, `marked`, `fflate`, `uplot`), awaiting thenable results, appending results to `history`, and returning `argument` when the driver returns `[null, argument]`.

When a component returns a typed array (`ArrayBuffer.isView(value)`), the linker converts it with `Array.from(value)` before pushing to `history`. Thus `fflate`'s `strToU8` and `gzipSync` `Uint8Array` results reach Kotlin as plain JS arrays (`history[3]` and `history[4]`), so step 4 passes `history[3]` straight as `data.bytes`, and the final snapshot reports `history[3].length` and returns `history[4]`.

## Component invocation order

1. **`idb` (`history.length == 0`)**: serializes each `input.records` document via `JSON.stringify(document)` into `{ id, group: "documents", value }` and builds the three-step `idb` script (`openDB` creating store `rows` with `keyPath: "id"` and index `byGroup` on `group`; `readwrite` batch of `put`s with `done: true`; `readonly` `openCursor` walk with `rest: [["continue"]]`; every step/request setting `catch: false`).
2. **`minisearch` (`1`)**: filters `history[0].log` for `"cursor"` entries, extracts each entry's `value` string, parses each with `JSON.parse<dynamic>(it)`, and requests `addAll` + `search` (`input.query`) with `fields: ["title", "body"]` and `storeFields: ["body"]`.
3. **`marked` (`2`)**: reads `history[1].results[1].hits`, selects `hits[0].body` (or `""` when empty), and requests `{ op: "parse", markdown }`.
4. **`fflate` `strToU8` (`3`)**: joins `stored(history)` with `joinToString("\n")` and requests `{ op: "strToU8", text }`.
5. **`fflate` `gzipSync` (`4`)**: passes `history[3]` as `data.bytes` with `{ level: 6, mtime: 0 }`.
6. **`uplot` (`5`)**: forwards `input.chart` directly to the Kotlin/JS `uplot` port.
7. **Snapshot (`else`)**: returns `[null, [ids, history[2], history[4], history[3].length, history[5]]]`.

## Kotlin/JS-specific details

- Uses `kotlin.js.Json` and `kotlin.js.json(...)` to construct plain JS objects and standard Kotlin `Array`s (`arrayOf`, `Array(n) { ... }`).
- Uses host `JSON.stringify` and `JSON.parse` so document key insertion order is preserved in the gzip archive bytes.

## Deviations, module state, and accommodations

- **Deviations from `reference.mjs`**: none; typed-array conversion (`Array.from`) of `fflate` results happens in the shared composite linker loop.
- **Module state**: none.
- **Performance accommodations**: none.
