# Explorer: Dart port

Hand-written Dart composition driver for the Offline Data Explorer application (`libraries/explorer/CONTRACT.md`), compiled with `dart2js` and linked by `languages/composite/build.mjs` with the five Dart component ports (`idb`, `minisearch`, `marked`, `fflate`, `uplot`).

## Layout

| File | What it is |
| --- | --- |
| `src/main.dart` | Step-function driver (`run`, `persist`, `stored`, `hits`, `command`) and JS interop helpers (`items`, `asArray`, `record`, `field`, `stringify`, `parse`), publishing `run` on `globalContext['__matrixExports']` |
| `components.lock.json` | Content-addressed lock of the five Dart component source inventories, compiled artifacts, toolchain, and composite linker |

## Boundary and composite loop

The composite linker (`languages/composite/build.mjs`) drives `run([input, history])` in a loop, invoking each requested Dart component (`idb`, `minisearch`, `marked`, `fflate`, `uplot`), awaiting thenable results, appending results to `history`, and returning the final snapshot when the driver returns `[null, snapshot]`.

At the linker boundary, any typed-array component return (`ArrayBuffer.isView(value)`) is converted with `Array.from(value)` before being appended to `history`. Because `fflate`'s `strToU8` and `gzipSync` return `Uint8Array`s, step 4 receives `history[3]` as a plain JS number array and passes it straight as `gzipSync`'s `data.bytes`, and the final snapshot reads `items(history[3]).length` and returns `history[4]` directly as the archive byte array.

## Component invocation order

1. **`idb` (`history.length == 0`)**: wraps each `input.records` item into `{ id, group: "documents", value: JSON.stringify(doc) }` and emits the three-step `idb` script (`openDB` creating store `rows` with `keyPath: "id"` and index `byGroup` on `group`; `readwrite` transaction batch of `put`s with `done: true`; `readonly` transaction `openCursor` walk with `rest: [["continue"]]`; every step/request setting `catch: false`).
2. **`minisearch` (`history.length == 1`)**: extracts cursor `value` strings from `history[0].log` (filtering `entry[0] == "cursor"`), parses each with `JSON.parse`, and runs `addAll` followed by `search` (`input.query`) with `fields: ["title", "body"]` and `storeFields: ["body"]`.
3. **`marked` (`history.length == 2`)**: reads `history[1].results[1].hits`, selects `hits[0].body` (or `""` when empty), and invokes `{ op: "parse", markdown }`.
4. **`fflate` `strToU8` (`history.length == 3`)**: joins the stored cursor `value` strings with `"\n"` and invokes `{ op: "strToU8", text }`.
5. **`fflate` `gzipSync` (`history.length == 4`)**: invokes `{ op: "gzipSync", data: { bytes: history[3] }, options: { level: 6, mtime: 0 } }`.
6. **`uplot` (`history.length == 5`)**: forwards `input.chart` directly to the Dart `uplot` port.
7. **Snapshot (`default`)**: returns `[null, [hitIds, history[2], history[4], items(history[3]).length, history[5]]]`.

## Dart-specific details

- Uses `dart:js_interop` and `dart:js_interop_unsafe` (`JSAny?`, `JSObject`, `JSArray`, `JSString`, `JSNumber`).
- Uses host `@JS('JSON.stringify')` and `@JS('JSON.parse')` so document key insertion order is preserved in the gzip archive bytes, matching the contract and reference.
- `items()` materializes JS arrays by indexed property access into `List<Value>`, and the `Iterable<Value>.asArray` extension constructs JS arrays for component arguments.

## Deviations, module state, and accommodations

- **Deviations from `reference.mjs`**: none; the `Array.from` conversion of `fflate`'s `Uint8Array` results happens in the shared composite linker loop rather than at the end of `run`.
- **Module state**: none.
- **Performance accommodations**: none.
