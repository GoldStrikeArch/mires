# Offline data explorer

Explorer composes the idb, MiniSearch, marked, fflate and uPlot matrix ports for the selected language. The reference invokes the same five pinned npm adapters. Each call creates a temporary IndexedDB database and persists documents with string IDs, titles and Markdown bodies. A later document with the same ID replaces the earlier value. The app walks the resulting records in primary-key order, indexes their titles and bodies, searches for the supplied query and renders the first hit's stored Markdown. A search with no hits renders the empty string.

The database is one idb script (`libraries/idb/CONTRACT.md`, "Input"), in three steps:

1. `openDB` of the logical database `explorer` at version 1, whose upgrade creates the store `rows` with key path `id` and its index `byGroup` on `group`;
2. a `readwrite` transaction on `rows` with one `batch`: a `put` on `tx.store` of `{ id, group: 'documents', value: JSON.stringify(document) }` for each document, in input order, then `done`;
3. a `readonly` transaction on `rows` with an `openCursor` walk of `tx.store` that moves with `continue` at every position.

Every step and request writes `catch: false` out, as the idb corpus does. The stored documents are the JSON texts of the walk's cursor entries in log order: `output.log.filter(entry => entry[0] === 'cursor').map(entry => entry[3].value)`. The idb port closes and deletes the database before it settles, so Explorer adds no cleanup of its own.

The archive contains each surviving document's JSON on its own line in cursor order, UTF-8 encoded then gzip-compressed at level 6 with a fixed timestamp. The app draws the supplied chart using the uPlot frame contract. The result contains the ordered hit IDs, selected HTML, exact gzip bytes, uncompressed byte count and chart geometry, in that order. The benchmark persists and indexes 96 documents before compressing the archive and drawing 320 samples. The corpus also exercises empty stores, missing hits, primary-key updates, Markdown and Unicode.

## The composite boundary

Each language's driver is a step function. The linker's generated `run` (`languages/composite/build.mjs`) calls the driver's `run([input, history])`, which returns `[component, argument]` for the next component call or `[null, snapshot]` when it is done. The linker calls that component's `run(argument)`, awaits the result when it is a promise, appends it to `history` and calls the driver again. The calls, in order, are idb, MiniSearch, marked, fflate `strToU8`, fflate `gzipSync` and uPlot.

One step of that loop converts a value. A typed-array result is appended as `Array.from(value)`, a plain array of numbers. fflate's two results are `Uint8Array`s, so the drivers read `strToU8`'s bytes as an array, pass that array as `gzipSync`'s `data.bytes` (the fflate contract's `{ bytes: [...] }` input) and return the archive as an array, where the reference passes the `Uint8Array` itself and returns `Array.from(archive)`. The conversion is the same JavaScript for every language and runs inside the timed `run`. No other value is converted: the other results reach the driver as the component returned them.

## Building and updating the components

From the repository root, run `node experiments/library-matrix/tools/build.mjs --library explorer --language rust`. The builder compiles the native app driver and checks each same-language component against its source inventory and the driver’s toolchain. It can reuse a matching standalone build. If receipts or artifacts are absent, stale or from another toolchain, it compiles the unmodified component source in private staging. Rollup links snapshots of the verified modules. The resulting readable and production ES modules contain the driver and all five component implementations. GopherJS has already optimized its modules. Its link step disables Rollup tree shaking to preserve the labelled breaks in generated Go control flow.

Each language's `components.lock.json` pins the five component source inventories, the active driver toolchain, the actual linked readable and production artifacts, and the common linker. A build compares these with the actual components before and after linking. To adopt reviewed component or linker changes, run `node experiments/library-matrix/tools/lock-components.mjs --library explorer --language all`, then rebuild Explorer. Use a language ID in place of `all` to update one target. Both commands accept the standard `--matrix` and `--receipts` overrides. Static sizes and the matrix report reject linked rows when a linked source, the linker or the application artifact differs from its lock. The source statistics include the driver and linked components. In particular, Rust Explorer builds MiniSearch with its current mires backend. The historical MiniSearch artifact and its original receipt remain available for the standalone row.

After building the application, run `node verify.mjs chrome` or `node verify.mjs firefox` from this directory. Optional language arguments select individual artifacts. The verifier runs six overlapping calls with the same primary keys, checks the search and rendered HTML, independently decompresses every archive with Node zlib, verifies document order and content, and checks that the page and its IndexedDB inventory return to their original state.


The package scripts `npm run check` and `npm run bench` validate the live component sources before and after calling the existing measurement workers. To select Explorer directly, use `node experiments/library-matrix/tools/qualify.mjs check --library explorer` or replace `check` with `bench`.

Document key order is observable in the gzip bytes. The timing fixture uses lexical key insertion order so the checker reading the corpus file and the benchmark receiving it through WebDriver serialize the same records. The independent verifier transmits its inputs as JSON text and checks exact decompressed bytes for documents constructed in both key orders.

Chart regression cases cover empty aligned series, replacing a populated chart with an empty one, restoring its data, and fractional physical sizes. Each benchmark input varies both the document collection and the chart coordinates.
