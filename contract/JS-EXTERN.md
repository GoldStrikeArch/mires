# JS-EXTERN: foreign call shapes

This document defines the JavaScript-level behavior of `#[js_extern]` declarations alongside the machine-readable test vectors in `fixtures/js-extern/vectors.json`. It describes each call shape, how neighboring shapes differ, how the attribute encodes descriptors, and which parts of the design remain open.

## Why a contract harness is needed

Three pieces must agree on what a `#[js_extern]` declaration emits: the compiler backend that lowers descriptors to JavaScript, Rust crates writing bindings against JS libraries, and the harness that checks compiled output against the expected call traces.

Many lowering bugs stay hidden against a real library. `new Chart(el, cfg)` and `Chart(el, cfg)` both draw a chart if the library tolerates calling its constructor as a plain function; `chart.resize(800)` and `chart.resize(800, undefined)` both resize; a nullable wrapper that checks only `undefined` looks fine until a library returns `null`. Pinning each shape against a recording fake turns those distinctions into testable traces.

## The recording fake

`fixtures/js-extern/chart-lib.mjs` is a checked-in recording fake modeled on how Chart.js is called, with no DOM dependency and no code derived from Chart.js. Like `harness/trace.mjs`, it executes to completion and leaves an ordered, normalized trace of every JavaScript operation performed on it.

| Binding | Shape | Notes |
| --- | --- | --- |
| `Chart` | Named export, constructor | `new Chart(target, config)`. Calling without `new` records the attempt and throws `TypeError`. |
| `default` | Default export, constructor | Same constructor exposed as a distinct function object so the imported binding is observable. |
| `Chart.register(plugin)` | Static method | Called through a dotted scope path on the export. |
| `Chart.version` | Static property | Property read at the same scope. |
| `chart.update(data)` | Instance method | One argument, no return value. |
| `chart.resize(width, height)` | Instance method | Second argument is optional; the fake branches on `arguments.length`. |
| `chart.destroy()` | Instance method | Zero arguments. |
| `chart.getPoint(index)` | Instance method | Returns a point in range and `null` out of range. |
| `chart.data` | Instance property | Returns a nested view so multi-step property paths can be tested. |
| `chart.data.datasets` | Indexed collection | Supports numeric index reads and `.length`. Out-of-range reads return `undefined`. |
| `chart.title` | Instance property | Readable and writable. |

Two design choices in `chart-lib.mjs` make otherwise invisible bugs show up in traces:

- **Two distinct constructor objects.** Real libraries frequently ship both named and default ESM exports. Making `Chart` and `default` separate function objects lets every trace entry record `via`, proving which import binding the descriptor selected.
- **Disagreeing out-of-range sentinels.** `getPoint(99)` returns `null` while `datasets[9]` returns `undefined`. Both appear in real libraries, and testing them side by side catches wrappers that check only one sentinel.

Trace labels are computed when each record is written. Instances are numbered in first-appearance order, and property views are labeled by path from their instance (for example `chart#0.data.datasets`), keeping diffs legible.

## Reference drivers and vectors

`fixtures/js-extern/drivers.mjs` contains one JavaScript reference driver per vector. Running each driver against `chart-lib.mjs` generates the expected trace in `vectors.json`, so every expected trace reflects what that JavaScript expression actually executes.

Because a recording always matches the code that produced it, each driver also carries explicit assertions about the shape it tests. The harness verifies two whole-suite properties on every run: every required shape has at least one vector, and no vector's assertions pass on any other vector's recording (all 272 pairs are checked for mutual distinguishability).

Compiled Rust output does not need to match the driver's exact syntax, only its recorded trace.

## Call shapes

### `new`

Constructs an instance. Vectors: `new-named`, `new-default`, `new-without-new`.

Calling the constructor without `new` throws a `TypeError` naming the missing `new`, matching standard ES classes; `new-without-new` records that failure explicitly. `new-named` and `new-default` differ only in the recorded `via` binding.

### `send`

Calls a method on an instance. Vectors: `send-one-arg`, `send-zero-args`, `send-trailing-omitted`, `send-trailing-undefined`.

Traces record `argc` separately from the argument values because libraries that branch on `arguments.length` distinguish an omitted argument from an explicit `undefined`. `send-trailing-omitted` and `send-trailing-undefined` record that difference directly (comparable to Melange's trailing-`undefined` rule). `send-zero-args` verifies that a zero-argument call passes an empty argument list (`argc: 0`), with no stray `undefined`.

### `get` and `set`

Reads or writes a property. Vectors: `get-property`, `get-scoped`, `get-static`, `set-property`.

A `get` performs a property lookup with no function call. Multi-segment paths such as `chart.data.datasets` are written as one declaration and emitted step by step, so each intermediate getter executes and appears in the trace. `get-static` uses the same mechanism rooted at a module binding; `set-property` tests assignment through the same path machinery.

### `index`

Reads an element from an indexed collection. Vectors: `index-in-range`, `index-length`, `index-out-of-range`.

Unlike `get`, the key is a runtime value recorded as a number. `length` is read with a plain `get` on the collection, keeping loops over host collections expressible from Rust without treating the collection as opaque.

### `call`

Calls a function reached through a scope path, such as `Chart.register(plugin)`. Vector: `call-static`.

## Return wrappers

`nullable-some` and `nullable-none` cover nullable returns (`null` when out of range). Combined with `index-out-of-range` (which yields `undefined`), these vectors ensure `#[js(nullable)]` treats both `null` and `undefined` as `None`.

## Attribute encoding

`js-extern-macro/src/descriptor.rs` is the single source of truth for encoding and decoding descriptors. The proc-macro crate compiles it directly, and the backend includes the same file via `#[path = "../../js-extern-macro/src/descriptor.rs"]`, preventing format drift between the two crates.

### Descriptor grammar

Each `#[js_extern]` item lowers to a marker function carrying its descriptor in `link_section`, letting `codegen_fn_attrs` transport it across crate boundaries:

```text
rcgjs.ext.<version>.<shape><flags>.<slots>.<module-len>,<name-len>.<module><name>
```

Example: `rcgjs.ext.3.n-.-.8,5.chart.jsChart` encodes `new Chart(..)` imported from `"chart.js"`.

| Field | Meaning |
| --- | --- |
| `rcgjs.ext.` | Prefix matched by the backend when intercepting `#[js_extern]` markers. |
| `<version>` | Currently `3`, parsed first so the decoder knows how many fields follow. Version `2` (the same grammar without `<slots>`) is still accepted. Unknown versions fail with an explicit version mismatch error. |
| `<shape>` | Single character: `c` (call), `n` (new), `s` (method / send), `g` (get), `t` (set), `i` (index), `j` (index-set). |
| `<flags>` | Sequence of flag letters, or `-` for none: `n` (nullable return), `g` (global root). `encode` writes flags in canonical order; `decode` accepts any order. |
| `<slots>` | Per-slot host contract, or `-` when unannotated. Otherwise a comma-separated list of `<key><letters>` entries in slot order (`<key>` is a decimal parameter index or `*` for the return value). Carrier letters: `b` baseline, `h` host string, `v` native vec, `s` slice view, `c` scalar. Observation letters: `r` reads, `d` identity, `y` bytes, `m` mutates, `t` retains, `o` owns, `x` throws, `p` suspends. |
| `<module-len>,<name-len>` | Decimal byte lengths of `<module>` and `<name>`. |
| `<module><name>` | Concatenated module specifier and JS path, split using `<module-len>`. |

Two more examples show the rest of the syntax:

- `rcgjs.ext.3.gn.-.0,5.width` is a nullable `get` of `width` rooted at argument 0 (the first `g` is the shape; `n` is the flag; empty module and absent `g` flag select the argument root).
- `rcgjs.ext.3.s-.1hcr,2mt.0,5.write` is a method `write` whose parameter 1 is read as a host string or scalar (`hcr`) and parameter 2 is mutated and retained (`mt`). Unmentioned slots (such as parameter 0) fall back to classification from their Rust types, matching version 2 behavior.

An empty `<name>` on an `index` or `index_set` descriptor indexes directly into the receiver with no intermediate property walk.

Version `3` added the `<slots>` field (a structural grammar change); version `2` added the `g` flag so older decoders reject it with a version diagnostic instead of an unknown-flag error. Descriptors are ephemeral build artifacts, so no persistence migration is needed.

### The three roots

Every descriptor resolves against one of three roots, represented by a `Root` enum in the backend:

| Root | How it is declared | Emitted target |
| --- | --- | --- |
| Argument 0 | Member shape with no module and no `global` flag | Operates on the first call argument, consuming it as the receiver |
| Module binding | Module set on the block or item | Emits `import { <first segment> as <local> }`, then walks the remaining path |
| Global scope | `#[js(global, ...)]` (or `call`/`new` with no module) | References the global identifier directly with no import |

Before the explicit `Root` enum existed, checking `module.is_empty()` left member shapes with no way to name a global: `#[js(get = "dash.status")]` read property `status` off argument 0 (or `undefined.status` for a zero-arg function) without raising an error. Making `global` explicit eliminates that ambiguity, along with three rules:

1. `call` and `new` default to the global root when no module is given, so declarations like `Math.max` or `new EventSource(url)` need no extra flag.
2. `global` clears a block-level `module = "..."`, letting browser globals sit inside a module block. Combining `#[js(global, module = "x")]` on the same item is a compile error.
3. `index` stays rooted at its first argument even inside a module block (matching `#[js(index = "data.datasets")]` in the chart fixture), unless `global` is set explicitly. This rule is documented on `Descriptor::root`.

### Length prefixes instead of delimiters

Module specifiers commonly contain `.`, `/`, `-`, `@`, and `:` (`chart.js`, `@scope/pkg`, `./local.js`, `node:fs`), while dotted JavaScript paths contain `.`, `$`, and `_`. Any delimiter drawn from those characters would require escaping, and escaping bugs silently mis-decode into the wrong call. Byte lengths require no escaping and let the decoder validate every input strictly.

Unit tests in `descriptor.rs` cover this directly: `a_specifier_full_of_delimiters_survives` round-trips `a.b,c-d/e`, while `every_malformed_descriptor_is_an_error_and_not_a_guess` and `a_length_that_splits_a_character_is_refused` check six distinct error paths (short payload, non-numeric length, missing comma, unknown flag, unknown shape, and UTF-8 boundary split). Plain text was chosen over a binary encoding such as postcard + base64 so descriptors stay diffable in fixtures, readable in error messages, and version-checked before parsing the rest.

### Lowering rules

1. **Module imports and `default`.** A `#[js_extern(module = "...")]` block supplies the default module for its items, overridable per item with `module = "..."`. Only the first segment of the JS path is imported (`#[js(call = "Chart.register")]` imports `Chart` and accesses `.register`). Bindings are interned by `(module, name)`, so two modules exporting `Chart` bind distinct locals while two crates importing the same `Chart` share one. The default export is written `default` and emitted as `import { default as ... }`; `examples/extern-tests/01_chart.rs:49-55`, `01_chart.js.expected`, and `scripts/extern-check.mjs` verify both named and default `new` bindings.
2. **Step-by-step property walks.** Dotted paths are emitted one member lookup at a time so every intermediate getter executes and appears in the trace.
3. **Exact arity.** Arguments pass through according to the Rust signature after consuming the receiver, index key, or assigned value. Nothing is padded with `undefined` and nothing is stripped, so a two-parameter Rust signature lowers to `send-trailing-omitted`; producing `send-trailing-undefined` requires declaring a third parameter and passing `undefined`.
4. **Nullable returns.** `#[js(nullable)]` stores the call result in a temporary so side effects run once, then tests `== null` so both `null` and `undefined` map to `None`. Declaring `nullable` on a setter is a compile error, and lowering `nullable` into a non-`Option` destination produces a link-time zombie.
5. **Indexed collections.** `index` emits numeric indexing (`coll[i]`) and `length` is bound as a `get`, keeping standard Rust iteration loops available.
6. **Three-way error split.**
   - Invalid declarations (too few parameters, `nullable` on a setter, invalid JS path, missing name on a binding root, duplicate or missing `#[js(...)]`) fail at compile time with a spanned macro error. Arity is also checked in the backend.
   - Undecodable descriptors or lowering mismatches (malformed descriptor text, missing operand, `set` without a property name, `nullable` with a non-`Option` destination) produce a reachability-gated link-time zombie naming the descriptor.
   - Mismatches against the target JavaScript library (declaring `new` on a plain function or naming a nonexistent export) surface as runtime `TypeError`s, as recorded in `new-without-new`.

### Global root tests

`fixtures/js-extern/vectors.json` drives the module-scoped chart fake and does not yet include global-root vectors. The backend tests all eight global shapes separately through `examples/extern-tests/02_globals.rs` and `scripts/globals-check.mjs` (17 checks, including verifying that the emitted text contains no `import` statements). When global vectors join `fixtures/js-extern/`, the planned IDs are `new-global`, `call-global`, `call-global-path`, `get-global`, `set-global`, `index-global`, `nullable-global`, and `new-global-without-new`.

## Byte buffers

Because `#[js_extern]` passes arguments unmarshalled, passing a Rust `&[u8]` directly hands JavaScript the compiler's internal `{ buf, off, len }` slice record instead of a `Uint8Array`. Use `mires_js::JsUint8Array` to cross the boundary in bulk by copy:

- `from_slice` copies Rust bytes into a fresh `Uint8Array` in one host operation.
- `copy_to_slice` and `to_vec` read the view's length from `%TypedArray%.prototype` internal slots and copy bytes back in one bulk operation.

Neither side shares backing storage after the copy. See `CONTRACT.md` ("Byte buffers cross by copy") for the full specification, and `examples/extern-tests/03_bytes_bulk.rs` + `scripts/extern-check.mjs` for the tests.

The slot observation flag `y` (`bytes`) is unrelated to marshalling: it informs the representation planner that the host observes a value's underlying bytes, pinning that edge to the baseline carrier without converting values.

## Running the harness

From `contract/`:

```sh
node --import ./harness/register-loader.mjs harness/check-js-extern.mjs
```

This compares the committed `vectors.json` against live driver runs and verifies all assertions and pairwise non-confusability checks. Pass `--record` to regenerate `vectors.json` (which `run-all.mjs` uses so driver edits surface as fixture diffs).

To test a compiled Rust module against these vectors:

1. Compile a module whose `#[js_extern]` declarations target `chart-lib.mjs`.
2. Resolve its import specifier to `fixtures/js-extern/chart-lib.mjs` (the same loader-mapping pattern `run-trace.mjs` uses) without modifying the compiled output.
3. Execute the operation corresponding to a vector's `id`.
4. Compare `__trace()` with the vector's `trace` using `compareTrace` from `harness/check-js-extern.mjs`.

`compareTrace` checks traces strictly position by position (unlike `compare-trace.mjs`, which uses longest-common-subsequence alignment for independent runtime calls), because reordering operations within a single foreign call changes its semantics.

## Open questions

- **Import specifier resolution.** Descriptors pass module strings verbatim into `import ... from "<specifier>"`, while the harness resolves specifiers via a loader hook. Standardized build configuration, import-map conventions, or bundler integration are still undecided.
- **Static factories returning instances.** `Chart.register` returns nothing; no vector yet exercises a static `call` whose return value is a new handle used with subsequent instance methods.
- **Callbacks.** Passing a compiled Rust closure to a JavaScript API (such as an event handler on a chart) is outside this contract and tested separately.
