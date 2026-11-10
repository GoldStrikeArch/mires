# rcgjs-object

The rustc-free half of the compiler, shared by the `rustc_codegen_js` producer (the codegen dylib)
and the `jsc-link` consumer. It depends on `serde`, `postcard`, `miniz_oxide`, `base64` and
`structurizer` only, so the linker, its tests, and any tool can read and write objects, print
JavaScript and parse options without a nightly toolchain.

Two things live here: the **JIR-S schema and the object format**, and the **modules the compiler
never needed rustc for** — the JavaScript AST and printer, the minifier, the item-local AST passes,
the item model, the source map, the `.d.ts` writer, the frozen v1 late IR, and the parsed options.
Anything that touches a `TyCtxt` stays in `backend/`; anything that reads an object file rather than
producing one lives in `jsc-link/`.

## What is here (Phase 1 W1a + W2)

`src/jir/`:

| module | contents |
| --- | --- |
| `semantic.rs` | JIR-S: `Body`, `BodyKind::{Function, Static, Vtable}`, locals, blocks, statements, rvalues, operands, places, terminators, `Call`/`Callee`/`HostPayload`, constants and `ConstTree`. v1 `late_ir.rs` schema 11 with `String` → `StrId`, `Statement.index` removed, and the additions the plan lists. |
| `types.rs` | `TypeTable` (crate-scope, sorted by `TypeId`) of `TypeDescriptor { kind, layout, meta, js }`, `VariantDescriptor`, `FieldDescriptor`/`FieldKey`, `Niche`, `SigTable`/`SigDescriptor`, and the tcx-free query helpers (`is_indirect`, `needs_clone`, `peeled`, `enum_repr`, `field`, `int_info`, `int_repr`, `unsized_tail`, `byte_element`, `pointer_form`, `is_zst`, `variants`, `variant_with_discriminant`). |
| `origin.rs` | `OriginTable`/`Origin`/`SourceFact`: diagnostic only, outside body equality. |
| `coroutine.rs` | Generic source blocks, typed control-flow edges, and yield sites captured before runtime MIR lowering. The intern section records bare drop locals, direct callee identities, source-local liveness and conservative move-path initialization too; separate monomorphic proofs are required for emission. |
| `interns.rs` | `StrId`, `BytesId`, `SigId` and the `Interns` builder (dedup via `BTreeMap`, stable ids). |
| `decline.rs` | The one `Decline` enum with a stable `code()` and a `Display` reproducing every v1 reason string. |
| `verify.rs` | `Body::verify`, `Body::verify_origins`, `TypeTable::verify`. |
| `hash.rs` | `Hash128`: SipHash-1-3 with a 128-bit output and fixed keys — the one digest for content addressing, section checksums, and (W4) body identity. Never `DefaultHasher`. |
| `codec.rs` | `Codec::Deflate` (miniz_oxide, raw deflate) plus the postcard `encode`/`decode` helpers; bodies and origins are deflated one blob at a time. |
| `summary.rs` | `ItemSummary` (the eagerly read S2 index entry: `SemanticKey`, `body_hash`, `Availability`, `deps`, `abi`, kind/linkage/order, blob refs, `ItemClass`), and the `ItemClass` bitset classifier over a `Body`. |

`src/object.rs`: object format 27: `Header`, the section table, `Writer`/`read`/`Object`, and
the item metadata the summary index shares with the producer (`ItemKind`, `Linkage`,
`SourceOrder`, `ItemMap`). See the module documentation for the layout; the format spec table there
is the one the docs quote.

## The rustc-free modules (W3)

| module | contents | what stayed in `backend/` |
| --- | --- | --- |
| `jsast.rs` | The JavaScript AST, its builders, the printer, source-map mapping emission, and the `structurizer::Payload` impl that says what a `Stmt` costs to duplicate. | nothing |
| `minify.rs` | Local and item renaming, the compact print style. `shorten_locals` is a no-op without `js-minify`; `shorten_locals_for_measurement` is the same renaming with that check left out, for measuring one item on a clone. | nothing |
| `optimizer.rs`, `sroa.rs` | The item-local AST passes `JsItem::new` runs. | nothing |
| `item.rs` | `JsName`, `ItemKind`, `Linkage`, `Zombie`/`FlatZombie`, `OptimizationInfo`, `JsItem`, `LinkItem`/`LinkJir`, `ObjectIdentity`, `normalized_emit_args`, `write_object`/`read_object`, `write_program`, `prologue`. | `ZombieLog` (renders a `Span` into a `loc`) and `object_identity(sess)`. |
| `late_ir.rs` | The frozen v1 schema-11 types (`LateIrAnalysisSummary`, `LateBody` and everything under it), `merge`, the validators, the postcard round trip. | the `Encoder`, `standard_operation`, `type_metadata`, the carrier-bridge encoder, and the three tcx constructors, now free functions `late_ir::{function, vtable, static_item}`. |
| `baseline_shadow.rs` | The non-emitting replay/round-trip check and its summary. | nothing |
| `opts.rs` | `Opts`, the enums, `parse_args`, the `Diagnostics` trait, the process-global `set`/`get`. | `SessionDiagnostics` and `init(sess)`. |
| `sourcemap.rs` | The version 3 map builder. It takes the source text as a map rather than reading a `SourceMap`; the dylib puts it in the manifest. | nothing |
| `dts.rs` | The `Dts` value and `module_dts`, the writer. | `signature(tcx, ..)` and the type walk that produces a `Dts`. |
| `ptr.rs` | The pointer *shape*: the field names and the constructors/accessors that build a slot, a fat slice, a fat `dyn`. | every reader and writer that consults a layout. |
| `manifest.rs` | The `Manifest` the dylib writes and `jsc-link` reads. Here rather than in `jsc-link` so the dylib never depends on the consumer — which is what keeps the sysroot cache key indifferent to a linker edit. | nothing |

`examples/jir-payload.rs`: the payload tool `scripts/jir-payload.sh` runs (per-section bytes, blob
dedup, inflated against deflated sizes) over rlibs and bare objects of either format.

## Schema rules

- `JIR_SCHEMA` (`jir/mod.rs`) versions everything in `jir/`. Objects written under another schema
  are rejected, never reinterpreted.
- Every `enum` is append-only: add variants at the end; never reorder, remove, or change a
  payload without a schema bump. Struct fields are positional too.
- Semantic content (`semantic.rs`, `types.rs`) carries identities only — no paths, spans, or text.
  `Body: Eq` is semantic equality. Origins live beside the body, never inside it.
- Determinism: `BTreeMap`/sorted `Vec` only; nothing here uses `DefaultHasher` or `HashMap`.
- Safe Rust only (`unsafe_code = "deny"` at the workspace).

## W1b additions (encoder round)

- `Callee::Nop` (appended): a call whose callee is empty drop glue (`ShimKind::DropGlue(_, None)`),
  which v1 emits as nothing; naming the glue item would dangle.
- `TypeTable::remove(id)` (helper, not schema): the encoder rolls back the descriptors one failed
  item inserted so no `Opaque` placeholder outlives an item.
- The `HostPayload`, `Call.track_caller`, `Callee::Foreign`/`Runtime`, and `VariantDescriptor::name`
  docs record the encoder's conventions.

## What the other workstreams add

- **W1b** (encoder, in `backend/src/jir/encode.rs`): `TypeDescriptor::of(tcx, ty)` porting
  `value.rs`/`ptr.rs`/`names.rs::field_name`/`vtable_entries`/`coroutine_layout`/`fn_sig`, and the
  MIR → `Body` encoder that fills `Interns`, `TypeTable`, and per-body `OriginTable`s.
- **W2** (landed): `jir/codec.rs`, `jir/hash.rs`, `jir/summary.rs`, `src/object.rs`.
- **W3** (landed): the std-only modules above, plus the `jsc-link` crate that consumes them.
- **W4** (`jir/summary.rs`): `SemanticKey`, normalized `body_hash`, and the report/JSON migration
  onto `Decline`.

## JIR-R (Phase 2 W6a)

`src/jir/repr.rs` is the representation-level SSA form the lowerer builds from a JIR-S body and
prints JavaScript from: `Func { item, sig, params, values[ValueDef { ty, carrier, origin }],
cells[CellDef { ty, boxed, origin }], blocks[RBlock { id, params, ops, term }] }`, `Op { result,
origin, kind: OpKind }`, `RTerm`, `Root::{Value, Cell}`, `Proj`, `RAssertMessage`, `Carrier`,
`FuncBuilder`, `Dominators`, and `Func::verify(&TypeTable)`. The module documentation is the
contract: value semantics, what the verifier checks, and the Phase 2 restrictions (`Baseline`
carriers only; intrinsics are `OpKind::Intrinsic`, never `Call`). Decisions beyond the plan's
sketch, each recorded on the item:

- `Field`/`SetField` name a field structurally (`variant: Option<u32>, field: u32`), never by
  JavaScript key; the key is `TypeTable::field(base_ty, variant, field)` at lowering time, so
  a later carrier can lay the same value out differently.
- `Call` carries `function: Option<ValueId>` for the `Callee::Indirect` operand, and keeps
  `host: Option<HostPayload>` beside the callee (a host call still names the marker instance);
  `OpKind::Host` is reserved for a host operation with no Rust callee.
- `Aggregate` has `active_field` (unions) and `Repeat { v, count }` exists (a `[v; N]` is not
  `N` fields).
- `Drop { root, path, glue }` takes a place path like a borrow, since MIR drops projected places.
- `CellDef` has an `origin` (the MIR local's), so a cell's `let` is named like a value's.
- Block arguments alias (they are moves); only an explicit `Move` op ends a live range for the
  verifier's use-after-move check.

`src/jir/interp.rs` holds the two interpreters (`run_body` for JIR-S, `run_func` for JIR-R) over
one abstract `Heap` of `Value`s, with `Trap`, `Event`, `Outcome`, `Hooks`. Every SSA value, cell,
and MIR local is a heap object, so `BorrowView`/`Reference` produce `Value::Ptr(Location)` into
it and the two evaluations of one program agree on results and traces (`interp/tests.rs`).

## Baseline-carrier spec

What `Carrier::Baseline` means, type family by type family: the JavaScript shape v1 emits and the
JIR-R lowering must reproduce, so JIR items and v1-text fallback items interoperate in one program.
References are `file:line` in v1 (`spike/rustc-codegen-js`); every shape below was checked
against v1 source and, where one exists, an emitted golden under `examples/emit/*.js.expected` or
`examples/dom-tests/*.js.expected`. Items marked *code only* have no checked-in golden. The type
table already answers most of these questions (`JsFacts`); this section says what the answers
mean in JavaScript.

### Integers

- `i8 i16 i32 u8 u16 u32 isize usize` are JS numbers; `i64 u64 i128 u128` are `BigInt`s
  (`backend/src/value.rs:875-880`, `IntRepr`). `isize`/`usize` are 32 bits: the target is
  `wasm32-unknown-unknown` (`value.rs:888-899`, `scripts/build_sysroot.sh:72`).
- Every arithmetic result is truncated by `mask` (`value.rs:902-916`): `i32` `v | 0`; `u32`
  `v >>> 0`; narrower signed `v << (32-N) >> (32-N)`; narrower unsigned `v & ((1<<N)-1)`; BigInt
  `BigInt.asIntN(bits, v)` / `BigInt.asUintN(bits, v)`. Printer forms pinned at
  `backend/src/jsast.rs:2432-2444`. Golden `examples/emit/01_arith.js.expected`:
  `(x + y & 255) << (3 & 7) & 255` (u8) and `((Math.imul(a + b | 0, 3) | 0) << 2 | 0) ^ a | 0`
  (i32).
- Operators (`backend/src/rvalue.rs:382-526`): `Add`/`Sub` are `mask(l + r)`/`mask(l - r)`;
  `Mul` is `mask(Math.imul(l, r))` for 32-bit number repr and `mask(l * r)` otherwise; `Div`
  `mask(l / r)` (the mask truncates toward zero); `Rem` `mask(l % r)`; bit ops masked; `Shl`
  `mask(l << amount)`; `Shr` `mask(l >> amount)` when signed or BigInt, `mask(l >>> amount)` for
  unsigned numbers. `shift_amount` (`rvalue.rs:467-485`) converts a mismatched repr
  (`BigInt(rhs)`, `Number(BigInt.asUintN(32, rhs))`), then masks: BigInt `amount & (bits-1)n`,
  32-bit numbers unchanged, narrower `amount & (bits-1)`. Comparisons are `=== !== < <= > >=`
  (`rvalue.rs:424-429`), allowed only on ints, floats, bool, char, fn pointers, and primitive
  enums (`rvalue.rs:775-789`); pointer comparisons go through `__rt.ptr_eq`/`__rt.ptr_cmp`.
- `Cmp` (`rvalue.rs:305-343`): `l < r ? Less : (l === r ? Equal : Greater)` with the three
  `Ordering` values spelled by `enum_value`, which for the real `core::cmp::Ordering`
  (`#[repr(i8)]`, fieldless) is `-1`, `0`, `1` (*code only*).
- `AddWithOverflow`/`SubWithOverflow`/`MulWithOverflow` (`rvalue.rs::codegen_with_overflow`): a
  two-element array whose first element binds the exact result to a hoisted temporary and whose
  second reads it back, `[$t1 = mask($t0 = l op r), $t1 !== $t0]` — array elements evaluate left
  to right, so no closure is needed. A 32-bit product is not exact in a double, so its wrapped
  half is `Math.imul(l, r)` and only the flag compares against the rounded `l * r`:
  `[$t0 = Math.imul(a, b) >>> 0, $t0 !== a * b]` (an operand that is not a name or a literal is
  bound to a temporary by its first read). On the JIR lane a pair every use of which is a
  `Field` projection is never built (`jsc-link` `rvalue.rs::exact_overflow_pairs`): it is carried
  as the exact `l op r`, `.0` reads `mask(x)` and `.1` `mask(x) !== x`, except for a 32-bit
  product, which keeps the array. (The DOM goldens under `examples/dom-tests` predate this and
  still show the old arrow form, `(($w) => [mask($w), mask($w) !== $w])(l op r)`.)
- `Assert` (`backend/src/base.rs:707-723`, `857-901`): `if (!cond) { <panic> }` where the panic
  is the lang item call (`panic_bounds_check(index, len, <loc>)`,
  `panic_misaligned_pointer_dereference(required, found, <loc>)`,
  `panic_invalid_enum_construction(source, <loc>)`, otherwise the message's panic function) or,
  without a lang item, `__rt.js_abort("<message>")` with the strings at `base.rs:1022-1032`.
  Golden `examples/emit/14_pointers.js.expected:8-10`.
- Casts (`rvalue.rs:688-719`): number to number `mask_to(v)`; number to BigInt
  `BigInt.asIntN/asUintN(bits, BigInt(v))`; BigInt to BigInt `BigInt.as*N(bits, v)`; BigInt to
  number `Number(BigInt.as*N(bits, v))` (truncate first); `bool as N` is `c ? 1 : 0` /
  `c ? 1n : 0n`; `u8 as char` is the identity. Literals: numbers via `resign` (`value.rs:930-959`);
  BigInt with a trailing `n`. BigInt emission is *code only* (no golden uses it).

### Floats

- `f64` and `f32` are JS numbers (`value.rs:6-7`). `f32` `Add Sub Mul Div Rem` are each wrapped
  in `Math.fround(..)` (`rvalue.rs:442-457`); `f64` never. `Neg` is `-v` unrounded (`rvalue.rs:535`).
  `IntToFloat` and `FloatToFloat` round with `Math.fround` when the destination is `f32`
  (`rvalue.rs:552-559`, `792-797`); `f32 as f64` is the identity. `f32` literals are printed as
  the exact double of the f32 (`backend/src/constant.rs:179`), not frounded. `CONTRACT.md:121`
  ("f32 without fround") is stale; the code is authoritative. *code only* for `Math.fround`.
- Float to int (`rvalue.rs:726-747`): `__rt.f2i(v, min, max)` with literal bounds for number
  destinations (`runtime/shim.js:43-48`: NaN to 0, clamp, `Math.trunc`), `__rt.f2i_big(v, bits,
  signed)` for BigInt (`shim.js:53-62`). Int to float: `Number(v)` for a BigInt source, else the
  value, then `float_round`.

### `bool`, `char`, unit, zero-sized types, `!`

- `bool` is `true`/`false`; `BitAnd`/`BitOr`/`BitXor` on `bool` are `&&`/`||`/`!==`
  (`rvalue.rs:433-440`). `bool` is direct, so a borrowed `bool` local is boxed.
- `char` is a JS number, the scalar value (`constant.rs:172`); direct; compares with `===`.
- Unit and every ZST are `undefined` (`value.rs:13`, `rvalue.rs:35-37`, `constant.rs:115`); ZSTs
  are indirect and never cloned. One inconsistency to decide deliberately: a fieldless struct read
  out of constant memory prints as `{}` (`constant.rs:255-269`; golden
  `13_topcoat_for.js.expected:72` `{ TAG: "Err", _0: {} }`).
- `!`: a diverging call is followed by
  `throw new Error("rustc_codegen_js: diverging call returned")` (`backend/src/abi.rs:262-267`);
  `Unreachable` prints `throw new Error("rustc_codegen_js: entered unreachable code")`
  (`backend/src/emit.rs:122-126`; golden: every `default:` in `examples/emit/18_enum_shapes.js.expected`).

### Tuples, arrays, closures, structs

- Tuple `(A, B)` is `[a, b]`; `()` is `undefined` (`rvalue.rs:220-226`). `[T; N]` is a JS array
  (`rvalue.rs:205`). A closure environment is a JS array of upvars in capture order; a
  non-capturing closure's environment is `undefined` (golden `examples/emit/12_closure.js.expected`).
- A struct is an object keyed by `Namer::field_name` (`backend/src/naming.rs:294-304`): a numeric
  field name gets `_` (`_0`, `_1`), a field literally named `TAG` becomes `TAG_`, anything else is
  `jsast::sanitize_ident` (`jsast.rs:1726-1739`: keep `[A-Za-z0-9_$]`, replace the rest with `_`,
  prefix `_` for empty/digit-leading/reserved). `FieldKey::Name`/`Index`/`Transparent` in the type
  table are exactly these (`value.rs:787-851`). Golden `examples/emit/09_struct_fields.js.expected:3`
  `o = { left: { a: 1, b: 2 }, right: { a: 3, b: 4 } };`.
- A union is an object with the one written field's key (`rvalue.rs:256-263`), unless
  transparent. Transparent aggregates (`value.rs:93-161`) are their one non-ZST field: a union
  with one non-ZST field, a `repr(transparent)` struct, `Box<T, ZST alloc>`, a packed newtype
  with its field's layout. So `MaybeUninit<T>` is `T` (`uninit()` is `undefined`, except
  `MaybeUninit<[T; N]>::uninit()` is `new Array(N)`, `rvalue.rs:811-820`), `NonNull<T>` is the
  pointer, `NonZero<u32>` a number, `Box<T>` its pointer.
- Coroutines (`value.rs:829-843`, `CONTRACT.md:1632-1656`): one object; upvars keyed by index
  (`g[0]`, ...); saved locals under `$s<CoroutineSavedLocal index>` (the type table's
  `saved_local_keys`), never a per-state position; `TAG` is the state index. *code only*.
- `Repeat` (`rvalue.rs:109-147`): `new Array(N).fill(v)` when the element needs no clone, else
  `Array.from({ length: N }, () => <clone>)`.
- `needs_clone` (`value.rs:573-586`): after peeling, an enum unless primitive, an ADT unless
  `Box`, a non-empty tuple, arrays, closures; never references, raw pointers, `&str`, ZSTs.
  `Operand::Copy` clones when `needs_clone`; `Operand::Move` never (`backend/src/place.rs:553-568`).
- `clone_expr` (`value.rs:603-689`) is structural, never a spread: struct `{ a: <clone a>, .. }`
  field by field; tuple `[ .. ]`; array `v.slice()` or `v.map(($e) => <clone>)`; closure `[ .. ]`
  over upvars; anything else `Object.assign({}, v)`; a transparent-union path guards with
  `v !== undefined ? <clone> : undefined`. Enums (`value.rs:700-758`): `Object.assign({}, v)`
  when no payload needs a clone or there are more than 8 variants, else a per-variant conditional
  chain on `v.TAG` with the last variant as the fallback. Golden `13_topcoat_for.js.expected:294`.

### Enums

- `EnumRepr` (`value.rs:218-333`): fieldless with an integer `repr` and a direct tag layout is
  the discriminant as a plain number (`Number`); fieldless without a repr is the variant name as a
  bare string (`Tag { fieldless: true }`); anything with a payload is `{ TAG: "Name", ..fields }`,
  fieldless variants included (`Option<T>` is `{ TAG: "None" }` / `{ TAG: "Some", _0: v }`,
  `Result` `{ TAG: "Ok", _0: v }`); a coroutine is `{ TAG: <state index>, .. }` (`State`). Niche
  layouts still get the object form. `TAG` is the literal `"TAG"` (`value.rs:211`). Payload keys
  are `field_name`: tuple variants `_0`, `_1`; struct variants their real names. Golden
  `examples/emit/18_enum_shapes.js.expected`: `"Pos"`, `66`, `{ TAG: "Nothing" }`,
  `{ TAG: "Num", _0: 7 }`, `{ TAG: "Pair", _0: 3, _1: 4 }`, `{ TAG: "Named", w: 5, h: 6 }`.
- Reading: `tag_expr` (`value.rs:406-411`) is `v` for primitive reprs and `v.TAG` otherwise;
  `discriminant_number` (`value.rs:422-460`) is a conditional chain
  `t === "A" ? 0 : t === "B" ? 1 : 2` for tagged enums. Which one a `Discriminant` rvalue gets is
  `tag.rs`: the tag when the local's only uses are `SwitchInt` scrutinees, the number otherwise
  (`rvalue.rs:168-196`). Switches use string cases for tagged enums and numbers for `Number` enums;
  or-patterns are fallthrough cases (`18_enum_shapes.js.expected`).
- `SetDiscriminant` (`base.rs:594-621`): `place.TAG = <tag>;` for object shapes (old payload keys
  stay), a whole-place write for primitive shapes. `Downcast` is a no-op on the expression
  (`place.rs:273-276`). Overwriting a whole enum through a reference is
  `__rt.overwrite(target, value)` (`backend/src/ptr.rs:264-269`, `shim.js:80-86`; golden
  `18_enum_shapes.js.expected` `overwrite`).
- Primitive enums are direct (`value.rs:327-333`): a borrowed local of one is boxed and `==` is
  `===`; payload enums are indirect and keep object identity.

### References and pointers

- Forms (`ptr.rs:18-31`, `CONTRACT.md:556-570`; constants `ptr.rs:66-78`, mirrored in this
  crate's `src/ptr.rs`): slot `{ buf, off }`; scaled slot `{ buf, off, sc }`; window slot
  `{ buf, off, w }`; fat slice `{ buf, off, len }`; fat dyn `{ ptr, meta }` (`meta` the vtable
  array); fat tail `{ ptr, meta }` (a struct with an unsized tail, `meta` the tail metadata); an
  address is a bare number (`null()` is `0`, `NonNull::dangling()` its alignment); a reference to
  an indirect pointee is the pointee's own object; `&str` is a JS string. A slot record is
  immutable: `add` builds a new record.
- `is_indirect` (`value.rs:543-561`): everything except int, uint, float, bool, char, fn pointer,
  reference, raw pointer, plus two overrides (a type crossing a transparent union is direct; a
  primitive enum is direct). `&T` to an indirect `T` is the object and `Deref` is a no-op
  (`place.rs:492-495`); `&T` to a direct `T` is a slot; every raw pointer to a sized non-ZST
  non-`str` pointee is a slot (`ptr.rs:1325-1364`).
- Building `&place` (`place.rs:467-551`) inverts the last projection: `A[k]` gives
  `{ buf: A, off: k }` (index captured by value), `A.f` gives `{ buf: A, off: "f" }` (*code
  only*), `p.buf[p.off]` folds back to `p`, an array pointee gives `__rt.array_ref(p, n)`, a
  fat pointee gives the pointer itself. `&x` of a boxed local is `{ buf: x, off: 0 }` (golden
  `14_pointers.js.expected:4`); `&arr[i]` is `{ buf: [10, 20, 30, 40], off: _10 }` (`:11`).
  A pointer is direct, so `&mut &mut T` is a slot and `*p = q` re-points (`value.rs:525-533`).
- Reads: `p.buf[p.off]` (`ptr.rs:154-192`); ZST pointee `undefined`; array pointee
  `__rt.read_array(p, N)`; unsized pointee the pointer itself. Writes (`ptr.rs:204-270`), in
  order: ZST pointee evaluates the value; unsized pointee `write_indirect`; array pointee
  `__rt.write_array(p, v)`; indirect pointee `__rt._put(base, key, v)` or `__rt.overwrite(target,
  v)`; else `p.buf[p.off] = v`. Golden `14_pointers.js.expected` `bump`:
  `target.buf[target.off] = target.buf[target.off] + by | 0;`.
- Offsets: strict `add` (`ptr.rs:304-328`) is `{ buf: p.buf, off: p.off + n }` (keeps `sc` for
  one-byte pointees), folded to `p.buf[p.off + n]` on read (golden `nth`); wrapping
  `__rt.offset(p, count, size)` (`shim.js:315-320`); `offset_from` is `a.off - b.off`,
  `__rt.offset_from(a, b)` for indirect pointees, `l - r` for ZST pointees (`ptr.rs:370-390`).
  Comparison `__rt.ptr_eq(a, b)` (`shim.js:281-291`, ignores `len`), `__rt.ptr_cmp(a, b, size) op 0`;
  address `__rt.addr(p, size)` (`shim.js:209-236`, a synthetic power-of-two granule base per buffer, reused after free).
- Casts (`ptr.rs:634-878`) go through the shim: `__rt.box(v)` (`shim.js:113-121`, `{ buf: [v],
  off: 0 }` memoized per object), `__rt.erase`, `__rt.scale`/`unscale`, `__rt.retype`,
  `__rt.unwindow`/`unwindow_slice`, `__rt.unref`, `__rt.thin`, `__rt.chunk_slice`,
  `__rt.retype_rc`, `__rt.copy`/`copy_nonoverlapping`, `__rt.write_bytes`,
  `__rt.compare_bytes` (line numbers in the shim: 672, 686, 716, 1194, 784, 632, 619, 763, 646,
  1232, 385, 442, 459, 492). Field reprojection emits `{ buf: p.buf[p.off], off: "<field>" }`
  per step (`ptr.rs:933-966`).
- `Box<T>` with a ZST allocator is exactly its pointer (`value.rs:78-82`, `CONTRACT.md:1253-1258`):
  a slot, a fat slice for `Box<[T]>`, a fat dyn for `Box<dyn Trait>`. Golden
  `examples/emit/17_alloc_shim.js.expected`: `boxed[0].buf[boxed[0].off] + 1 | 0`.
- Slices: `&[T]` is `{ buf, off, len }` over the backing JS array; `&[T; N]` to `&[T]` is
  `{ buf: <array>, off: 0, len: N }` (`backend/src/unsize.rs:174-188`); element read
  `s.buf[s.off + i]` (`place.rs:346-354`, `+ 0` folded); `Subslice` shares `buf`. Golden
  `13_topcoat_for.js.expected:80` `{ buf: _26.buf, off: _26.off, len: size }`.
- `str`: `&str` is a JS string; `PtrMetadata` is `__rt.str_len(s)`; `as_bytes` is
  `__rt.str_bytes(s)` returning `{ buf: <array of numbers>, off: 0, len }` (`shim.js:830-840`,
  4-entry memo whose identity nothing may rely on, `CONTRACT.md:825-864`); `as_ptr` is
  `{ buf: __rt.str_bytes(s).buf, off: 0 }`; `from_utf8_unchecked` is `__rt.bytes_str(buf, off,
  len)` (`shim.js:870-897`). Byte buffers are plain JS arrays, never `Uint8Array`. Goldens in
  `examples/dom-tests/19_text_conversion.js.expected:105,258,400,402,451`.
- Allocator (`shim.js:1283-1328`, `CONTRACT.md:1152-1270`): `__rt.alloc(size, align)` returns
  `{ buf: new Array(size), off: 0 }` registered as a fresh byte block; `alloc_zeroed` fills `0`;
  the block is retyped once at its first `*mut u8 -> *mut T` cast; `realloc` resizes the same
  array; `dealloc` empties it. `String` is a `Vec<u8>` block.

### Cells

- A local is a cell iff it is bare-borrowed (`&_5`, not `&_5.x`; `backend/src/uses.rs:173-175`)
  and its type is direct (`base.rs:209-216`). A `Drop` with real glue counts as a bare borrow
  (`uses.rs:262-265`). The cell is a one-element JS array: non-parameter `let x = [];`, parameter
  `x = [x];` in the prelude (`base.rs:430-440`); every read/write is `x[0]` (`place.rs:336-343`);
  `&x` is `{ buf: x, off: 0 }`. A boxed return place returns `_0[0]` (`emit.rs:119-121`). There is
  no `{ v }` cell and no accessor object (`CONTRACT.md:132-137`). Golden
  `14_pointers.js.expected:2-3` `let value = [], base, _10; value[0] = 1;`.
- Statics are module-level `let NAME = <value>` (`base.rs:125-134`), boxed as `[value]` when the
  type is direct (`base.rs:108-120`, *code only*), the bare value otherwise (golden
  `examples/dom-tests/14_topcoat_local.js.expected:11`); a reference to a boxed static is
  `{ buf: NAME, off: 0 }` (`constant.rs:470-475`).

### Functions, closures, `dyn`, vtables

- A reified fn pointer or fn-pointer constant is the emitted function's identifier
  (`rvalue.rs:604-621`, `constant.rs:445-448`); two reifications of one item are the same object,
  so fn pointers compare with `===`. A closure value is its environment array; the body is a
  top-level function whose first parameter is the environment (golden
  `12_closure.js.expected`). `ClosureFnPointer` wraps `call_once` in
  `($a0, ..) => target(undefined, $a0, ..)` (`rvalue.rs:622-649`).
- Virtual call (`abi.rs:246-256`, `327-329`): the receiver hoisted into `$tN`, then
  `$tN.meta[<absolute slot>]($tN.ptr, ..rest)`. Golden `19_text_conversion.js.expected:358`
  `$t0.meta[3]($t0.ptr, s)`. `dyn` drop: `if ($tN.meta[0] !== null) { $tN.meta[0]($tN.ptr); }`
  (`abi.rs:278-291`).
- Vtable (`backend/src/vtable.rs:70-89`): `const NAME = [<drop glue or null>, <size>, <align>,
  <method>, ..]` in `tcx.vtable_entries` order; `Vacant` is `null`; `TraitVPtr` is the nested
  vtable array. Golden `19_text_conversion.js.expected:13-14`. Unsize `&T -> &dyn Trait` is
  `{ ptr: <reference form of T>, meta: <vtable> }` (`unsize.rs:217-231`); `dyn -> dyn` upcast is
  `{ ptr: v.ptr, meta: v.meta[<supertrait slot>] }` or the value itself (`unsize.rs:196-215`).

### Calling convention

- Positional JS parameters in MIR argument order, the return local returned; no return pointer
  (`abi.rs:1-16`, `base.rs:368-381`). A unit function falls off its end (golden
  `examples/emit/11_unit_fn.js.expected` `function ..() {}`); an aggregate is returned as itself.
- `extern "rust-call"`: the callee declares `<name>$e0`, `<name>$e1`, .. and rebuilds the tuple in
  the prelude (`let args = [args$e0, args$e1];`, `base.rs:359-373`, `424-429`); the caller splats
  `pack[i]`, cloning each when `needs_clone` (`abi.rs:293-325`). Golden `12_closure.js.expected`
  `closure_0(f, [x][0])`.
- `#[track_caller]`: a trailing parameter `$loc` (`backend/src/intrinsics.rs:65`, `base.rs:377-379`);
  each call site appends either the enclosing `$loc` or a hoisted `const` location object
  (`intrinsics.rs:1643-1740`) keyed by the **actual `panic_location` lang item's field names**
  through `field_name` (the JIR-S encoder puts that type in the table whenever it records a
  `Call::track_caller`), values `file` string, `line` number, `col + 1` number. Goldens:
  `14_pointers.js.expected:9` `{ file: "..", line: 68, column: 17 }` (mini_core),
  `13_topcoat_for.js.expected:179` `{ filename: "alloc/src/raw_vec/mod.rs", line: 28, col: 5 }`
  (real `core`).
- Callees (`abi.rs:29-40`, `240-250`): `Direct` `name(args)`; `Runtime` `__rt.member(args)`;
  `Indirect` `expr(args)`; `Virtual` as above; `Nop` nothing. `Drop` calls the glue with the
  reference to the place (`base.rs:826-846`).
- Locals are declared once at the top of the function in one `let` list (`base.rs:414-459`).
  Reserved names: `$tN` hoisted temporaries, `$sN` structurizer scrutinee temporaries, `$aN`
  thunk parameters, `$w` waker parameter, `$e` clone parameter, `$c` captured environment,
  `$loc`, `<name>$eN` spread parameters.

### Panics, aborts, unwinding

- `__rt.js_abort(msg)` throws `Error("rust abort: " + msg)` (`shim.js:24-26`); the `abort`
  intrinsic is `__rt.js_abort("rustc_codegen_js: \`abort\` intrinsic")` (`intrinsics.rs:329-332`);
  `unreachable` and `Region::Unreachable` throw `"rustc_codegen_js: entered unreachable code"`; a
  surviving unwind edge throws `"rustc_codegen_js: unwinding is not supported"`
  (`emit.rs:127-131`). `catch_unwind` is supported under `-Cpanic=abort` only: the try function
  is called in the prelude and the intrinsic's value is `0` (`intrinsics.rs:444-457`). A zombie
  is a runtime abort standing in for an unsupported construct (`base.rs:314-317`).
