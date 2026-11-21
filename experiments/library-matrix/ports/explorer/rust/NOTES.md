# Explorer: Rust (mires) port

Hand-written `#![no_std]` Rust composition driver for the Offline Data Explorer application (`libraries/explorer/CONTRACT.md`), compiled with `mires` and linked by `languages/composite/build.mjs` with the five Rust component ports (`idb`, `minisearch`, `marked`, `fflate`, `uplot`).

## Layout

| File | What it is |
| --- | --- |
| `src/lib.rs` | `#![no_std]`, `#![no_main]`, `#![deny(unsafe_code)]` step-function driver (`persist`, `transaction`, `stored`, `hits`, and `#[js_export] pub fn matrix_run`) using `mires_js` |
| `components.lock.json` | Content-addressed lock of the five Rust component source inventories, compiled artifacts, toolchain, and composite linker |

## Boundary and composite loop

The Rust builder's facade re-exports `matrix_run` as `run`. The composite linker (`languages/composite/build.mjs`) calls `run([input, history])` in a loop, invoking each requested Rust component (`idb`, `minisearch`, `marked`, `fflate`, `uplot`), awaiting thenable results, appending results to `history`, and returning `argument` when the driver returns `[null, argument]`.

When a component returns a typed array (`ArrayBuffer.isView(value)`), the linker converts it with `Array.from(value)` before pushing to `history`. Thus `fflate`'s `strToU8` and `gzipSync` `Uint8Array` results arrive as plain JS arrays (`at(history, 3)` and `at(history, 4)`), so step 4 passes `at(history, 3)` straight as `data.bytes`, and the final snapshot reports `len(at(history, 3)) as f64` and returns `at(history, 4)`.

## Component invocation order

1. **`idb` (`len(history) == 0`)**: serializes each `input.records` document via host `JSON.stringify` into `{ id, group: "documents", value }` and builds the three-step `idb` script (`openDB` creating store `rows` with `keyPath: "id"` and index `byGroup` on `group`; `readwrite` batch of `put`s with `done: true`; `readonly` `openCursor` walk with `rest: [["continue"]]`; every step/request setting `catch: false`).
2. **`minisearch` (`1`)**: walks `history[0].log` for entries where `at(entry, 0).strictly_equals(s("cursor"))`, parses each entry's `value` string via host `JSON.parse` (`stored(history, true)`), and requests `addAll` + `search` (`input.query`) with `fields: ["title", "body"]` and `storeFields: ["body"]`.
3. **`marked` (`2`)**: reads `history[1].results[1].hits`, selects `hits[0].body` (or `""` when empty), and requests `{ op: "parse", markdown }`.
4. **`fflate` `strToU8` (`3`)**: collects the stored cursor `value` strings (`stored(history, false)`), joins them via host `Array.prototype.join` with `"\n"`, and requests `{ op: "strToU8", text }`.
5. **`fflate` `gzipSync` (`4`)**: passes `at(history, 3)` as `data.bytes` with `{ level: 6, mtime: 0 }`.
6. **`uplot` (`5`)**: forwards `get(input, "chart")` directly to the Rust `uplot` port.
7. **Snapshot (`_`)**: returns `[null, [ids, at(history, 2), at(history, 4), len(at(history, 3)) as f64, at(history, 5)]]`.

## Rust / mires-specific details

- Uses `mires_js::{JsValue, js_bindings, js_export, throw_value}` with bindings to `Reflect.get`, `Reflect.set`, `new Array`, `new Object`, array indexing, `.length`, `.push`, `.join`, `JSON.stringify`, and `JSON.parse`.
- Uses host `JSON.stringify` and `JSON.parse` so document key insertion order is preserved in the gzip archive bytes.
- Per `libraries/explorer/CONTRACT.md`, Rust Explorer links all five Rust components built with the active `mires` toolchain (including `minisearch` built with the current `mires` backend).

## Deviations, module state, and accommodations

- **Deviations from `reference.mjs`**: none; typed-array conversion (`Array.from`) of `fflate` results happens in the shared composite linker loop.
- **Module state**: none.
- **Performance accommodations**: none.
