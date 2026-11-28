# rcgjs-object

`rcgjs-object` is the `rustc`-free core crate shared by the `rustc_codegen_mires` backend dylib and the `jsc-link` linker. It depends only on ordinary crates (`serde`, `postcard`, `miniz_oxide`, `base64`, `structurizer`), so the linker, unit tests, and inspection tools can read and write objects, manipulate JIR, print JavaScript, and parse options without a nightly `rustc_private` toolchain.

Code that queries `TyCtxt` stays in `backend/`; code that links or optimizes whole programs lives in `jsc-link/`.

## JIR and object format (`src/jir/`, `src/object.rs`)

| Module | Contents |
| --- | --- |
| `jir/semantic.rs` | JIR-S (`Body`, `BodyKind::{Function, Static, Vtable}`, locals, blocks, statements, rvalues, operands, places, terminators, `Call`/`Callee`/`HostPayload`, and `ConstTree`). |
| `jir/repr.rs` | JIR-R SSA representation (`Func`, `ValueDef`, `CellDef`, `RBlock`, `Op`/`OpKind`, `RTerm`, `Root`, `Proj`, `Carrier`, `FuncBuilder`, `Dominators`, and `Func::verify`). |
| `jir/types.rs` | Crate-scoped `TypeTable` sorted by `TypeId` (`TypeDescriptor`, `VariantDescriptor`, `FieldDescriptor`/`FieldKey`, `Niche`, `SigTable`) and `TyCtxt`-free query helpers (`is_indirect`, `needs_clone`, `peeled`, `enum_repr`, `field`, `int_info`, `unsized_tail`, `pointer_form`, `is_zst`). |
| `jir/origin.rs` | `OriginTable`, `Origin`, and `SourceFact` (diagnostic metadata kept outside semantic equality). |
| `jir/interns.rs` | `StrId`, `BytesId`, `SigId`, and deterministic `BTreeMap`-backed `Interns`. |
| `jir/summary.rs` | Eagerly read `ItemSummary` index entries (`SemanticKey`, `body_hash`, `semantic_hash`, `Availability`, `deps`, `abi`, `ItemClass`), `NormalizedBody`, and duplicate/alias merge rules. |
| `jir/conflict.rs` | First-difference comparator, `Cause` classifier, and JSON/text conflict reporting shared by `jsc-link` and `examples/jir-conflicts.rs`. |
| `jir/decline.rs` | Unified `Decline` enum with stable codes and formatted diagnostic strings. |
| `jir/verify.rs` | Structural verifiers: `Body::verify`, `Body::verify_origins`, `TypeTable::verify`. |
| `jir/hash.rs` | `Hash128` (SipHash-1-3-128 with fixed keys) for content addressing, checksums, and body identity. |
| `jir/codec.rs` | Raw deflate (`miniz_oxide`) + `postcard` serialization helpers for per-item blobs. |
| `jir/dependencies.rs` | Scans decoded JIR bodies for potential item dependencies (including cleanup-only callees). |
| `jir/interp.rs` | Reference interpreters (`run_body` for JIR-S, `run_func` for JIR-R) over a shared abstract heap; gated behind `#[cfg(any(test, feature = "interp"))]`. |
| `object.rs` | Object format `29` (`Header`, section table, `Writer`, `read`, `Object`). See `object.rs` module docs for the binary section layout (S1–S7). |

## Shared compiler modules

| Module | Contents |
| --- | --- |
| `jsast.rs` | JavaScript AST, builders, `Visit`/`VisitMut` traits, printer, source-map emission, and `structurizer::Payload` cost impl. |
| `minify.rs` | Local and internal-item renaming plus compact printing (`shorten_locals` and measurement helpers). |
| `optimizer.rs`, `sroa.rs` | Item-local JS AST simplification and scalar replacement helpers. |
| `item.rs` | `JsName`, `ItemKind`, `Linkage`, `Zombie`, `OptimizationInfo`, `JsItem`, `LinkItem`/`LinkJir`, `ObjectIdentity`, and `normalized_emit_args`. |
| `opts.rs` | `Opts`, `Profile`, `OptimizerMode`, `OptimizerPass`, `EcmaTarget`, `Modules`, `StrRepr`, and `-Cllvm-args` parsing. |
| `manifest.rs` | Postcard `Manifest` (`MANIFEST_VERSION = 6`) passed from the backend dylib to `jsc-link`. |
| `artifact.rs`, `lane_report.rs` | Linker-owned artifact manifests and per-item lane availability reports. |
| `sourcemap.rs` | Source Map v3 builder. |
| `dts.rs` | TypeScript `.d.ts` representation and writer. |
| `ptr.rs`, `host.rs`, `evaluation.rs` | Canonical pointer field/constructor helpers, host binding metadata, and constant evaluation helpers. |

Inspection examples in `examples/`:

- `examples/jir-payload.rs` (`scripts/jir-payload.sh`) — reports per-section sizes, blob dedup, and compression ratios for objects and rlibs.
- `examples/jir-conflicts.rs` — inspects multi-object item duplicates and prints normalized diffs.

## Schema rules

- `JIR_SCHEMA` (`jir/mod.rs`, currently `25`) and `FORMAT_VERSION` (`object.rs`, currently `29`) version serialized objects strictly. Mismatched objects fail with a rebuild diagnostic.
- Postcard enums and structs are positional: adding or changing variants or fields requires bumping the schema version.
- Semantic types (`semantic.rs`, `types.rs`) carry identities only, with no source paths, spans, or diagnostic strings. `Body: Eq` represents semantic equality, and `OriginTable` lives beside the body.
- All serialized maps and sets use `BTreeMap` or sorted `Vec` for deterministic output (`DefaultHasher` and `HashMap` are excluded from serialized data).
- Workspace lint `unsafe_code = "deny"` applies to the whole crate.

## JIR-R design notes

`src/jir/repr.rs` defines the SSA form `jsc-link` optimizes and lowers to JavaScript:

- `Field` and `SetField` identify fields structurally (`variant: Option<u32>, field: u32`) and resolve JS property names later via `TypeTable::field`, allowing passes to change carrier layouts without rewriting field references.
- `Call` carries `function: Option<ValueId>` for indirect calls and `host: Option<HostPayload>` beside the callee for host markers; `OpKind::Host` represents a host operation with no Rust callee.
- `Aggregate` records `active_field` for unions, and `Repeat { v, count }` represents `[v; N]` compactly.
- `Drop { root, path, glue }` accepts a projection path to match MIR drops of projected places.
- Block parameters represent SSA moves; explicit `Move` ops end live ranges for the verifier's use-after-move checks.

## Baseline JS carrier reference

This summarizes what `Carrier::Baseline` means in emitted JavaScript for each Rust type family on the 32-bit `targets/mires32-unwind.json` target. Optimized builds may replace private values with native carriers (such as `ByteStr`, `NativeString`, `NativeVec`, SoA columns, or niche enums); exported and pinned boundaries use these baseline shapes.

### Integers and floats

- `i8`/`i16`/`i32`/`u8`/`u16`/`u32`/`isize`/`usize`: JS numbers (`isize`/`usize` are 32-bit). Arithmetic masks results (`| 0` for `i32`, `>>> 0` for `u32`, shift pairs for narrow signed types, bitmasks for narrow unsigned types, `Math.imul` for 32-bit multiply).
- `i64`/`u64`/`i128`/`u128`: JS `BigInt` masked with `BigInt.asIntN` / `BigInt.asUintN` (optimized internal 64-bit hot paths may also use unboxed 32-bit lo/hi pairs via `jsc-link/src/lower/js/i64_pair.rs`).
- `f64` and `f32`: JS numbers; `f32` arithmetic and conversions round with `Math.fround(...)`. Float-to-int saturating casts use `__rt.f2i` (numbers) and `__rt.f2i_big` (`BigInt`).
- `AddWithOverflow` / `SubWithOverflow` / `MulWithOverflow`: carried as exact arithmetic when only `.0` or `.1` projections are used, or as a two-element `[wrapped, overflowed]` array when used whole.

### `bool`, `char`, ZSTs, and `!`

- `bool` is a JS boolean (`true`/`false`), `char` is its numeric Unicode scalar value, and `()` / zero-sized types are `undefined`.
- Diverging calls and `Unreachable` terminators throw an explicit runtime error if reached.

### Aggregates and enums

- Tuples `(A, B)`, arrays `[T; N]`, and closure environments are JS arrays (`()` and non-capturing closure environments are `undefined`).
- Structs are plain objects keyed by sanitized field names (numeric names prefix `_`, a field named `TAG` becomes `TAG_`). Transparent wrappers (`repr(transparent)`, `MaybeUninit<T>`, `NonNull<T>`, `NonZero<T>`, single-non-ZST-field structs, and `Box<T, Global>`) peel to their inner representation.
- Copying a direct scalar or reference aliases by value; copying an aggregate clones structurally (field-by-field object literals or array `.slice()`/`.map(...)`).
- Fieldless enums with an integer `#[repr(...)]` lower to their discriminant number; other fieldless enums lower to the variant name string; enums with payloads lower to `{ TAG: "Variant", ...fields }` (unless specialized by `plan/enum_repr.rs` to a null-niche or integer tag). Coroutines use `{ TAG: <state>, ... }` with `$s<N>` saved-local keys.

### References, pointers, slices, `str`, and cells

- Direct values (primitives, pointers, references, primitive enums) become `{ buf, off }` slots when address-taken; their locals are boxed in one-element arrays (`let x = [];`) so `&x` is `{ buf: x, off: 0 }`.
- Indirect values (structs, arrays, tuples, payload enums) alias by JS object identity, so `&T` is the object itself.
- Raw pointers to sized non-ZST values use `{ buf, off }` (or scaled `{ buf, off, sc }` / window `{ buf, off, w }` slots). Fat slices are `{ buf, off, len }`, and fat `dyn`/unsized-tail pointers are `{ ptr, meta }`.
- Baseline `&str` is a JS string (`js-str=utf16`), with `__rt.str_bytes` / `__rt.bytes_str` bridging UTF-8 byte views; release profiles default to `js-str=bytes` (`ByteStr`).
- Heap blocks allocated by `__rt.alloc` start as byte blocks and retype on their first `*mut u8 -> *mut T` cast; `dealloc` empties and poisons freed blocks.

### Calls, `dyn`, and unwinding

- Functions take positional MIR arguments and return the return local directly. `extern "rust-call"` unpacks tuple arguments (`<name>$e0`, ...); `#[track_caller]` appends a trailing `$loc` parameter.
- Vtables are `const` arrays `[drop_glue_or_null, size, align, method0, ...]` in `tcx.vtable_entries` order; virtual calls dispatch via `v.meta[slot](v.ptr, ...args)`.
- Host exceptions unwind through `try`/`catch` blocks emitted from `rustc`'s MIR unwind edges; Rust panics call the crate's `#[panic_handler]` and abort via `__rt.js_abort`.
