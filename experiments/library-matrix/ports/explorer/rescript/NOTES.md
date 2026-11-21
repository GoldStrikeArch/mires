# Explorer: ReScript port

Hand-written ReScript composition driver for the Offline Data Explorer application (`libraries/explorer/CONTRACT.md`), linked by `languages/composite/build.mjs` with the five ReScript component ports (`idb`, `minisearch`, `marked`, `fflate`, `uplot`).

## Layout

| File | What it is |
| --- | --- |
| `src/Index.res` | Host value bindings and step-function driver (`transaction`, `persist`, `stored`, `hits`, `cmd`, `run`) |
| `components.lock.json` | Content-addressed lock of the five ReScript component source inventories, compiled artifacts, toolchain, and composite linker |

## Boundary and composite loop

The composite linker (`languages/composite/build.mjs`) calls `run([input, history])` in a loop, invoking each requested ReScript component (`idb`, `minisearch`, `marked`, `fflate`, `uplot`), awaiting thenable results, appending results to `history`, and returning `argument` when the driver returns `[null, argument]`.

When a component returns a typed array (`ArrayBuffer.isView(value)`), the linker converts it with `Array.from(value)` before pushing to `history`. Thus `fflate`'s `strToU8` and `gzipSync` `Uint8Array` results arrive as plain JS arrays (`at(h, 3)` and `at(h, 4)`), so step 4 passes `at(h, 3)` straight as `data.bytes`, and the final snapshot reports `length(at(h, 3))` and returns `at(h, 4)`.

## Component invocation order

1. **`idb` (`length(h) == 0`)**: serializes each `input.records` document via `@scope("JSON") @val external stringify` into `{ id, group: "documents", value }` and builds the three-step `idb` script (`openDB` creating store `rows` with `keyPath: "id"` and index `byGroup` on `group`; `readwrite` batch of `put`s with `done: true`; `readonly` `openCursor` walk with `rest: [["continue"]]`; every step/request setting `catch: false`).
2. **`minisearch` (`1`)**: filters `history[0].log` via JS `Array.prototype.filter` (`at(entry, 0) === box("cursor")`), extracts each entry's `value` string, parses each via `JSON.parse`, and requests `addAll` + `search` (`input.query`) with `fields: ["title", "body"]` and `storeFields: ["body"]`.
3. **`marked` (`2`)**: reads `history[1].results[1].hits`, selects `hits[0].body` (or `""` when empty), and requests `{ op: "parse", markdown }`.
4. **`fflate` `strToU8` (`3`)**: joins `stored(h)` via JS `Array.prototype.join` with `"\n"` and requests `{ op: "strToU8", text }`.
5. **`fflate` `gzipSync` (`4`)**: passes `at(h, 3)` as `data.bytes` with `{ level: 6, mtime: 0 }`.
6. **`uplot` (`5`)**: forwards `get(input, "chart")` directly to the ReScript `uplot` port.
7. **Snapshot (`_`)**: returns `[Nullable.null, [ids, at(h, 2), at(h, 4), length(at(h, 3)), at(h, 5)]]`.

## ReScript-specific details

- Uses an opaque `type value` with zero-cost `%identity` boxing (`box`) and direct ReScript externals (`@new`, `@get_index`, `@set_index`, `@get`, `@send`, `@scope("JSON") @val`).
- Uses `Nullable.null` for JS `null` values (`store`, `index`, and the final `[null, snapshot]` terminator).
- Uses host `JSON.stringify` and `JSON.parse` so document key insertion order is preserved in the gzip archive bytes.

## Deviations, module state, and accommodations

- **Deviations from `reference.mjs`**: none; typed-array conversion (`Array.from`) of `fflate` results happens in the shared composite linker loop.
- **Module state**: none.
- **Performance accommodations**: none.
