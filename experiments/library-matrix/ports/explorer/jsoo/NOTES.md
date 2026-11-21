# Explorer: js_of_ocaml port

Hand-written OCaml composition driver for the Offline Data Explorer application (`libraries/explorer/CONTRACT.md`), compiled with `js_of_ocaml` and linked by `languages/composite/build.mjs` with the five `js_of_ocaml` component ports (`idb`, `minisearch`, `marked`, `fflate`, `uplot`).

## Layout

`src/program.ml` is shared and byte-identical between `ports/explorer/jsoo/` and `ports/explorer/melange/`. Each port supplies its own compiler-specific `src/host.ml` and entry module.

| File | What it is |
| --- | --- |
| `src/program.ml` | Shared Explorer step-function driver (`persist`, `stored`, `hits`, `command`, `run`) written against `Host` |
| `src/host.ml` | `js_of_ocaml` interop (`Js.Unsafe` object/array/property/method helpers, `JSON.stringify`, `JSON.parse`, `Array.prototype.join`, primitive boxing) |
| `src/main.ml` | Entry module exporting `Program.run` via `Js.export "run" (Js.wrap_callback Program.run)` |
| `components.lock.json` | Content-addressed lock of the five `jsoo` component source inventories, compiled artifacts, toolchain, and composite linker |

## Boundary and composite loop

The composite linker (`languages/composite/build.mjs`) calls `run([input, history])` in a loop, invoking each requested `jsoo` component (`idb`, `minisearch`, `marked`, `fflate`, `uplot`), awaiting thenable results, appending results to `history`, and returning `argument` when the driver returns `[null, argument]`.

When a component returns a typed array (`ArrayBuffer.isView(value)`), the linker converts it with `Array.from(value)` before pushing to `history`. Thus `fflate`'s `strToU8` and `gzipSync` `Uint8Array` results arrive as plain JS arrays (`at history 3` and `at history 4`), so step 4 passes `at history 3` straight as `data.bytes`, and the final snapshot reports `length (at history 3)` and returns `at history 4`.

## Component invocation order

1. **`idb` (`length history = 0`)**: serializes each `input.records` document via host `JSON.stringify` into `{ id, group: "documents", value }` and builds the three-step `idb` script (`openDB` creating store `rows` with `keyPath: "id"` and index `byGroup` on `group`; `readwrite` batch of `put`s with `done: true`; `readonly` `openCursor` walk with `rest: [["continue"]]`; every step/request setting `catch: false`).
2. **`minisearch` (`1`)**: filters `history[0].log` for `"cursor"` entries, extracts each entry's `value` string, parses each via host `JSON.parse`, and requests `addAll` + `search` (`input.query`) with `fields: ["title", "body"]` and `storeFields: ["body"]`.
3. **`marked` (`2`)**: reads `history[1].results[1].hits`, selects `hits[0].body` (or `""` when empty), and requests `{ op: "parse", markdown }`.
4. **`fflate` `strToU8` (`3`)**: joins the stored cursor `value` array via host `Array.prototype.join` with `"\n"` and requests `{ op: "strToU8", text }`.
5. **`fflate` `gzipSync` (`4`)**: passes `at history 3` as `data.bytes` with `{ level: 6, mtime: 0 }`.
6. **`uplot` (`5`)**: forwards `get input "chart"` directly to the `jsoo` `uplot` port.
7. **Snapshot (`_`)**: returns `[null, [ids, at history 2, at history 4, int (length (at history 3)), at history 5]]`.

## js_of_ocaml-specific details

- All JS values remain opaque `Js.Unsafe.any` (`Host.value`) across the boundary; only the cursor tag comparison (`text (at entry 0) = "cursor"`) converts a JS string via `Js.to_string`.
- Host `JSON.stringify` and `JSON.parse` preserve document key insertion order in the gzip archive bytes.

## Deviations, module state, and accommodations

- **Deviations from `reference.mjs`**: none; typed-array conversion (`Array.from`) of `fflate` results happens in the shared composite linker loop.
- **Module state**: none.
- **Performance accommodations**: none.
