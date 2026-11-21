# Explorer: Elm port

Hand-written Elm 0.19 composition driver for the Offline Data Explorer application (`libraries/explorer/CONTRACT.md`), linked by `languages/composite/build.mjs` with the five Elm component ports (`idb`, `minisearch`, `marked`, `fflate`, `uplot`).

## Layout

| File | What it is |
| --- | --- |
| `src/Main.elm` | Stateless `Platform.worker` (`Program () () E.Value`) implementing the Explorer step function (`drive`, `persist`, `stored`, `hits`) over `Json.Decode` / `Json.Encode` with `request` and `response` ports |
| `js/index.js` | Minimal Elm/JS I/O-only host exporting `run(packet)`: initializes the single worker, sends `packet` on `request`, and returns the synchronous `response` value |
| `components.lock.json` | Content-addressed lock of the five Elm component source inventories, compiled artifacts, toolchain, and composite linker |

## Boundary and Elm host

All Explorer composition logic lives in `src/Main.elm`. `js/index.js` is an I/O-only port bridge and does no Explorer work:

- On first call, it starts `Elm.Main.init()` (whose model is `()`) and subscribes to `worker.ports.response`, storing each outgoing value in the module-level `reply` slot.
- On every `run(packet)`, it resets `reply = undefined`, sends `packet` through `worker.ports.request.send(packet)`, checks `if (reply === undefined)` (throwing if Elm did not reply synchronously), and returns `reply`.
- Synchronous port delivery is provided by the Elm builder's packaging (`languages/elm/build.mjs`).

The composite linker (`languages/composite/build.mjs`) drives `run([input, history])` in a loop, invoking each requested Elm component, awaiting thenable results, converting any typed-array return (`ArrayBuffer.isView(value)`) via `Array.from(value)` before pushing to `history`, and returning `argument` when the driver returns `[null, argument]`. Because `fflate`'s `strToU8` and `gzipSync` return `Uint8Array`s, `history[3]` and `history[4]` arrive in Elm as plain JSON arrays of numbers.

## Component invocation order

1. **`idb` (`List.length (items history) == 0`)**: serializes each `input.records` item with `E.encode 0 doc` (which delegates to JS `JSON.stringify`, preserving document key insertion order) into `{ id, group: "documents", value }` and emits the three-step `idb` script (`openDB` creating store `rows` with `keyPath: "id"` and index `byGroup` on `group`; `readwrite` batch of `put`s with `done: true`; `readonly` `openCursor` walk with `rest: [["continue"]]`; every step/request setting `catch: false`).
2. **`minisearch` (`1`)**: filters `history[0].log` for `"cursor"` entries, extracts each entry's `value` string, decodes it back to an opaque `E.Value` via `D.decodeString D.value`, and requests `addAll` + `search` (`input.query`) with `fields: ["title", "body"]` and `storeFields: ["body"]`.
3. **`marked` (`2`)**: reads `history[1].results[1].hits`, selects `List.head` `.body` (or `""` if empty), and requests `{ op: "parse", markdown }`.
4. **`fflate` `strToU8` (`3`)**: joins the stored cursor `value` strings with `String.join "\n"` and requests `{ op: "strToU8", text }`.
5. **`fflate` `gzipSync` (`4`)**: passes opaque `at 3 history` (already converted by the linker's `Array.from`) as `data.bytes` with `{ level: 6, mtime: 0 }`.
6. **`uplot` (`5`)**: forwards opaque `field "chart" input` directly to the Elm `uplot` port.
7. **Snapshot (`_`)**: returns `[null, [ids, at 2 history, at 4 history, List.length (items (at 3 history)), at 5 history]]`.

## Deviations, module state, and accommodations

- **Deviations from `reference.mjs`**: none; typed-array conversion (`Array.from`) of `fflate` results happens in the shared composite linker loop.
- **Module state**: `src/Main.elm` keeps no state (`model = ()`). `js/index.js` holds the single lazily-initialized `worker` instance and the transient synchronous `reply` slot reset on every call.
- **Performance accommodations**: opaque values (`input.chart`, `history[2]`, `history[3]`, `history[4]`, `history[5]`, parsed documents) stay `Json.Encode.Value` without deep decoding into Elm records.
