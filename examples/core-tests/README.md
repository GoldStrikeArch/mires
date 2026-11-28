# The `core` test suite

These tests are ordinary `#![no_std]` crates compiled against the real `library/core` and `library/alloc` built by `rustc_codegen_mires`, with no `mini_core`, `#![no_core]`, or hand-written lang items.

```sh
./scripts/build_sysroot.sh      # ~1 minute, once per option key
./scripts/test.sh               # runs examples/tests/ and then this suite
./scripts/test.sh 04_iter_range # run a single test
```

`scripts/build_sysroot.sh` copies `rust-src` into a keyed directory under `build/sysroot-cache/`, applies `patches/*.patch` (plus `patches/unwind/*.patch` in the default unwind mode), compiles `core`, `compiler_builtins`, and `alloc` with `-Zcodegen-backend`, and installs the resulting rlibs into the keyed sysroot. `scripts/compile_core.sh` then builds each test with `--crate-type cdylib`, invoking `jsc-link` for whole-program dead-code elimination and reachability-gated zombie checks.

Each test has an `.expected` file (exact stdout), a `.maxbytes` budget (to catch regressions in dead-code elimination), and—for `10_panic` and the `guard_*` tests—an `.expect_abort` marker requiring a non-zero exit. Expected outputs were checked by running the tests natively against `std`; the only intentional differences are the two 32-bit pointer-width checks (`06_num`'s `usize::BITS == 32` and `07_mem`'s `size_of::<&i32>() == 4`, matching `targets/mires32-unwind.json` and `wasm32-unknown-unknown` against 64-bit hosts).

## Test catalog

| Test | Coverage |
| --- | --- |
| `01_option` | Predicates, `unwrap`/`unwrap_or`/`unwrap_or_else`/`unwrap_or_default`, `map`/`and_then`/`filter`/`or`/`map_or`, `?`, `take`/`replace`/`as_mut`/`flatten`, `PartialEq`, and `Option<&T>` / `Option<bool>` niches |
| `02_result` | `is_ok`/`is_err`, `unwrap_or*`, `map`/`map_err`/`and_then`, `ok`/`err`, `?` with a custom error enum, `Result<T, ()>` |
| `03_cmp` | `Ordering` on primitives, `reverse`/`then`/`is_lt`, `min`/`max`/`clamp`, derived `PartialOrd`/`Ord` on structs and enums, hand-written `Ord`, generic functions over `Ord` |
| `04_iter_range` | `for`, `..=`, `step_by`, `rev`, `sum`/`product`/`count`/`fold`/`max`/`min`/`last`/`nth`, `map`/`filter`/`skip`/`take`/`take_while`/`skip_while`/`filter_map`/`chain`/`zip`/`enumerate`, `any`/`all`/`find`/`position`, manual `next`, `u64` ranges, `Range` as a value |
| `05_slice_index` | Array-to-slice coercion, `len`/`is_empty`, constant and computed indexing, `get`/`get_mut`/`first`/`last`/`last_mut`, mutation through `&mut [T]`, slice patterns including `rest @ ..`, arrays of structs, 2-D arrays |
| `06_num` | Type constants, `wrapping_*`/`checked_*`/`overflowing_*`/`saturating_*`, `count_ones`/`leading_zeros`/`swap_bytes`/`rotate_left`, `abs`/`signum`/`pow`/`next_power_of_two`, `div_euclid`/`rem_euclid`, the full Number/`BigInt` cast matrix, saturating float-to-int, `NonZero::new` |
| `07_mem` | `size_of`/`align_of`, `replace`/`take`/`swap`/`drop`/`forget`/`ManuallyDrop`, `Drop` order for locals, fields, early returns, and moves |
| `08_dyn` | `&dyn`/`&mut dyn`, overridden and default methods, supertraits, `dyn` in struct fields and arrays, `&mut dyn FnMut` |
| `09_closure` | `Fn`/`FnMut`/`FnOnce`, captures by reference and by value, `impl Fn` returns, `fn` pointers from items and non-capturing closures, closures inside `core` combinators |
| `10_panic` | `Option::unwrap` on `None` -> `core::panicking::panic` -> the crate's `#[panic_handler]` with `#[track_caller]` `Location`, followed by a non-zero exit |
| `11_fmt` | `write!` against `core::fmt`: format templates, `Display` for integers, `str`, and `char`, width/fill/radix in `Formatter`, explicit argument indices, and `Arguments::as_str` |
| `12_slice_iter` | `iter()`/`iter_mut()`/`for x in &xs`, `rev`/`nth`/`position`/`find`/`enumerate`/`zip`/`map().sum()`/`last`/`min`/`max`/`fold`, `len`/`as_slice` on partially consumed iterators, `split_at`, subslices, struct and zero-sized elements |
| `13_sort` | `sort_unstable`/`sort_unstable_by`/`sort_unstable_by_key` on `[i32]`, `[u64]`, and structs at lengths 0, 1, 2, 16, and 40, plus `swap`, `reverse`, `contains`, `binary_search`, `copy_from_slice`, `starts_with`/`ends_with`, `fill` |
| `14_str` | `len`/`is_empty` on ASCII and non-ASCII, `as_bytes` and indexing, `bytes()`, `chars()`, `char_indices()`, `==`/`!=`/`<`/`cmp`, `starts_with`/`ends_with`, `contains`, `find(char)` and `find(&str)`, `&s[a..b]` and `get(a..b)`, `trim`, `split(char)`, `from_utf8`, `escape_ascii`/`escape_debug`, `char::len_utf8` |
| `15_ptr` | `null`/`is_null`, `without_provenance` and `addr` round trips, `ptr::eq` on aliases, `offset`/`add`/`sub`/`offset_from`, `read`/`write`, `read_unaligned`/`write_unaligned`, `byte_add`/`byte_offset`/`byte_sub` over byte and non-byte pointees, overlapping `copy`, `copy_nonoverlapping`, `write_bytes`, `swap`, `&raw` on struct fields, `*const T` -> `*const u8` chains, 32- and 64-bit `from_le_bytes`/`to_le_bytes`, `[u8; N]`/`[i32; N]` equality |
| `19_float_fmt` | `Display`/`LowerExp` for `f32`/`f64`: precision, width, fill, sign, long forms of `f64::MAX`/`f64::MIN_POSITIVE`, `NaN`/`inf`/`-0`, `{:?}` on `char`, and `str::parse::<f64>()` |
| `20_alloc_box` | `Box` across all four shapes (scalar, mutated struct, trait object, boxed slice) and `Drop` running pointee glue before freeing |
| `21_vec` | `Vec` growth via `push`, in-place `realloc`, zeroed allocation via `vec![0; n]`, borrowing as `&[T]` for scalar and aggregate elements |
| `22_string` | `String` backed by `Vec<u8>`: byte-by-byte and piece-by-piece construction, decoding to `&str` on deref |
| `23_format` | `format!` streaming `core::fmt` into a growing heap buffer (width, fill, precision) plus the host string builder wrapped by `prelude::JsStr` |
| `24_enum_repr` | `core` enums across representations: fieldless `#[repr(int)]` (`Ordering`, `mem::Alignment`), `Option<T>` niches (`&T`, `NonZero`, `char`), whole-value writes (`take`/`replace`/`insert` through `&mut Option<T>`), `mem::discriminant` |
| `25_array_ref` | `&*p` for `[E; N]` pointees from a plain slot, an `as *mut [E; N]` window, and a slice record (matching `itoa::Buffer`, needed by `serde_json` integer formatting) |
| `26_async` | Coroutine state machines as enums: state discriminant tag, saved locals via `CoroutineSavedLocal`, construction, and manual polling via `block_on` |
| `27_dangling` | `NonNull::dangling()` wrapped in a slice so iterating an unallocated `Vec` works cleanly |
| `28_array_into_iter` | By-value `for x in arr` via `array::IntoIter` and `PolymorphicIter`'s unsized tail (`{ ptr, meta }`), from both ends and with `Drop` elements |
| `29_rc` | `Rc` and `Arc` for sized types (strong/weak counts, `get_mut`, `ptr_eq`, `Weak`, payload drop on final owner, `Rc<RefCell<T>>`) and unsized slices (`Rc<[u8]>`, `Rc<[u32]>`, `Rc<[String]>` via `from_iter`, `Drop` elements, `Arc<[T]>`, empty tail) |
| `30_hashmap` | `mires_js::collections::{HashMap, HashSet}`: all key classes including `BigInt` widths and `str`, `insert` returning previous values, struct mutation through `get_mut`, `remove`, snapshot iteration, set deduplication |
| `102_str_lines` | `str::Lines`, `split_once`, CRLF handling, and 1–4 byte `char` search and suffix matching via scalar byte views, rejecting impossible conversion, slice-index, aligned-word, and forward `Map` paths |
| `104_exact_typed_vec` | Exact/reserved `Vec<u32>` storage planning: width-aware cast capacity, straight-line push length proofs, capacity-observation and retained-view declines, `Uint32Array` selection, logical capacity, scalar slice consumers |
| `105_slice_iterator_cursor` | Checked cursor lowering for `Enumerate<Take<SliceIter<u8>>>` and `Enumerate<Skip<SliceIter<u8>>>` loops (empty, short, exact, overlong, dynamic, early-break), plus decline cases for signed/wider elements, reversed adapters, and observed iterator state |
| `114_zst_aggregate_drops` | Arrays and slices of zero-sized elements keep `undefined` storage while dropping each element once (locals, returns, parameters, `&mut [Z]`, `move` closures, `Vec<[Z; N]>`, `[[u8; 0]; N]`, zero-length arrays, partial moves, replacement, and unmaterialized `[Z; 2000]`); `scripts/cleanup-smoke.sh` tests host-throw cleanup |
| `115_zst_element_pointers` | All pointers to zero-sized elements share the container's single address (`&a[i]`, `a.as_ptr()`, `s.as_ptr()`, `s.first()`, iterator items, `add`/`wrapping_add`, `from_raw_parts`, `Vec<Z>` dangling pointer) under `ptr::eq`, `addr_eq`, and `as usize`, staying non-null and aligned (including 64-byte-aligned ZSTs) |
| `guard_as_chunks` | Verifies that `as_chunks` aborts at runtime via a guard instead of silently returning wrong elements |
| `guard_heap_retype` | Reading a written heap block at a second element size triggers `__rt.unscale` to throw |
| `guard_map_key` | Compile-time type error via the sealed `MapKey` trait when an aggregate is used as a host `Map` key |
| `guard_rc_str` | Unsized heap tail boundary: `Rc<str>::from` works (`433_str_tail_dst`), while `Rc::<str>::from_raw` on a bare `*const str` fails loudly |
| `guard_use_after_free` | `dealloc` poisons freed blocks so subsequent pointer casts fail explicitly |
| `guard_zeroed_retype` | Retyping a zeroed allocation to a type with no valid all-zero JS form (such as a fat pointer `&[T]`) throws |

`30_hashmap` and `guard_map_key` are the only tests here that link an extra crate beyond `core` and `alloc`. `scripts/test.sh` builds `mires-js` into an rlib and passes it via `--extern`; tests that do not reference it link nothing from it. `std`/`hashbrown` `HashMap` cannot run on the JS heap model because `hashbrown` reads one allocation at two element granularities (the exact boundary pinned by `guard_heap_retype`; see `CONTRACT.md`, "Host collections"), so `mires-js` wraps JavaScript's native `Map` and `Set`.

`12_slice_iter`, `14_str`, and `15_ptr` exercise the `{ buf, off }` pointer model. A pointer is a slot—a JS container plus an element-indexed key—so dereferencing produces an assignable place and pointer arithmetic returns a new slot (`CONTRACT.md` specifies the model; `15_ptr` tests its primitives). `14_str` also guards against an early bug where `*const u8` taken from a `&str` produced the string itself, causing `s.as_bytes().len()` to return `undefined`.

### Cyclic statics and chunk splitting

| Test | Coverage |
| --- | --- |
| `260_static_cycles` | Self-referential and mutually recursive statics initialize cleanly and preserve identity: pairs, 3-node rings, self-references, external pointers into cycles, leaf statics, hoisted long strings, static slices/arrays of references, reference-only boxed statics, and `const` tables naming cycle members |
| `261_static_cycle_cells` | Cycles with interior mutability: `AtomicU32` fields, `Cell`s in cyclic `static mut` pairs via raw pointers, and self-referential `static mut`s where writes through one path read back through another |
| `262_chunk_owned_statics` | Three-way chunk split: chunk-owned statics naming a shared static, chunk-owned statics naming another chunk's entry point, interior-mutable shared statics written from two chunks |
| `263_chunk_three_owners` | Three-way splits in both declaration orders: chains from chunk-owned to shared statics, cross-chunk static edges, shared statics naming a chunk entry point, hoisted strings in chunk-owned statics |
| `264_chunk_static_cycles` | Two-way split: shared static cycles mutated across chunks, single-chunk cycles and self-references, cycles naming another chunk's entry point |
| `265_chunk_static_entries` | Four-way split with two `static` entry points whose imports form a DAG |
| `266_chunk_static_entry_refusals` | Linker refusals for invalid splits: a `static` entry point mutually referencing a shared binding, and a split dividing a static cycle; splitting by functions alone succeeds |
| `267_static_cycle_arrays` | Cyclic array statics behave as real arrays: `.slice()` value copies, by-value passes, arrays of structs, runtime subslices, reversed iteration, `map`, 40-element tables, grids, static slices, and ZST statics |
| `268_static_cycle_proto_key` | `.expect_fail`: rejects a cyclic static containing a `__proto__` field at link time, since two-step initialization would drop it |

Tests with a `.chunks` companion file are also built split by `scripts/chunk-test.sh`. Each line lists `chunk:entry` pairs passed as `js-chunk=` options; every split runs entries in the order of the first line (matching `rust_entry`), so a single `.expected` file covers both unsplit and split builds. A `run:` line specifies which entries to invoke (static entries are exported, not called), and a `refuse: <pairs> -- <text>` line asserts that linking fails with `<text>` in stderr. `scripts/check.sh` runs this suite. See `CONTRACT.md` ("Statics that refer to each other", "Who owns what", and "Around a cycle of imports, only functions") for the formal rules.

## Known limits and why

Each limit below stems from either a deliberate representation constraint or a specific missing piece.

### 1. Reinterpreting a buffer at another element granularity

A slot can re-index a buffer, but it cannot synthesize elements of a different size. Viewing `[i32]` as `[[i32; 2]]` or raw `usize` words expects elements that do not exist in the JS array, and static types do not distinguish an owned `[E; N]` slot from a cast window over another buffer. Runtime checks in `__rt.chunk_slice` and `__rt.unscale` throw instead of returning corrupt values:

- `as_chunks`, `array_chunks`, `chunks_exact`, and `align_to` abort (`guard_as_chunks`).
- `memrchr`'s word-at-a-time path goes through `align_to`, so `rfind(char)` hits this guard. Forward `memchr` uses the scalar path in the patched sysroot, so `str::find(char)` and `split(char)` work at any length; `find(&str)` and `contains` use `StrSearcher`, which compares slices (`14_str`).
- Pointer casts between pointees whose sizes differ (when neither is 1 byte) are rejected as zombies naming both types (`examples/tests/zombie_reachable.rs`).

`ptr_mask` is permanently unsupported because JS pointer addresses are synthetic IDs, not memory addresses with manipulable low bits.

### 2. Formatting and sysroot patches

`write!` and `Display` work across integers, strings, chars, and floats (`11_fmt`, `19_float_fmt`, including non-exponential `f64::MAX`/`f64::MIN_POSITIVE`, `NaN`/`inf`/`-0`, `{:?}` on `char`, and `str::parse::<f64>()`).

`Debug` as a trait bound remains unsupported:

- `#[derive(Debug)]` and `Result::unwrap`/`Result::expect` coerce `E: Debug` to `&dyn Debug`, pulling every reachable `Debug` implementation into the build. Use `.ok().unwrap()`, `unwrap_or*`, or `match` instead.

Concrete `{:?}` on `char` works because `char::escape_debug` casts a buffer of one-byte `AsciiChar` enum values to `str`, and slices of one-byte enums decode directly to JS strings (`CONTRACT.md`, "`str` is a hybrid"; pinned at small scale by `examples/tests/16_byte_enum_str.rs`). That also unblocked `&s[a..b]`, `contains`, and `find(&str)` (`14_str`).

Several sysroot patches in `patches/` keep common `core` paths free of unreachable formatting or word-alignment machinery:

- `0001-panic-without-formatting.patch` replaces runtime formatting in `const_panic!`, `panic_bounds_check`, `panic_display`, `assert_failed`, and `str::slice_error_fail_rt` with fixed strings so bounds checks, `clamp`, `assert!`, and string slicing work without dragging in `core::fmt` or Unicode tables (`54_str_index_no_fmt`).
- `0004-char-searcher-byte-length.patch` casts `char::encode_utf8`'s length directly to `u8` (always `1..=4`) instead of keeping an unreachable `TryInto::expect` error path (`102_str_lines`).
- `0005-scalar-memchr.patch` keeps forward `memchr` on its safe scalar scan instead of the aligned-word path (reverse `memrchr` is untouched).
- `0006-char-searcher-nonpanicking-needle.patch` compares `CharSearcher` candidates and `char` suffixes through checked views with explicit length guards, avoiding unreachable `slice_index_fail` graphs.
- `0007-lines-direct-split.patch` drives `SplitInclusive` directly in `Lines::next` with safe byte checks for `CR? LF`, eliminating the forward `Map::next` adapter while preserving `Lines`' public representation and reverse iteration.
- `0012-char-searcher-scalar-prefix.patch` matches the 0–3 bytes preceding the final UTF-8 byte directly in forward `CharSearcher` instead of routing single-char searches through generic slice equality.

Additional patches in `patches/` (`0002`, `0003`, `0013`–`0023`) and `patches/unwind/` support the allocator, native string/vector markers, diagnostic items, and `-Cpanic=unwind` personality/nounwind annotations.

### 3. Unfinished items

Two gaps have known implementations waiting to be finished:

- `NonNull::dangling()` (`transmute::<Alignment, NonNull<T>>`): `Alignment` converts to an integer discriminant, while the pointer-direction transmute and `Option<NonZero<usize>>` niche required by `NonNull::addr` are still incomplete (`15_ptr` uses `without_provenance(align_of::<T>())`).
- `Ipv6Addr` parsing and formatting, which reinterprets `[u16; 8]` as `[u8; 16]` (hitting the granularity boundary in section 1).

#### Zombie census

Exact counts of remaining zombies in the built `core` and `alloc` rlibs live in one place: `CONTRACT.md` ("The zombie census in `core` and `alloc`"). None of the surviving entries is reachable from string handling, formatting, or slice operations, and `u8 as char` is lowered directly in `rvalue.rs` and `intrinsics.rs`.

### 4. Other unsupported features

- Allocations with non-slice unsized tails (`Rc<str>`, `Arc<dyn Trait>`, `core::ffi::CStr`, `core::wtf8::Wtf8`), plus `Rc::into_raw`/`from_raw` (which step backwards across the header with `byte_sub`). A heap block holds one uniform element type, so `Rc<[T]>`, `Arc<[T]>`, `Box`, `Vec`, `String`, and `format!` work (`20_alloc_box`–`23_format`, `29_rc`), whereas a header plus a JS string does not (`guard_rc_str`).
- Writing through `&mut str` (JS strings are immutable).
- `raw_eq` on aggregates other than scalar arrays, and `write_bytes` on aggregates.
- `TypeId` equality (no stable byte layout).
- `f16` and `f128` (absent from JavaScript).
- `simd_*` intrinsics (lowered as zombies).
- Multi-threaded atomics (atomic intrinsics lower to plain single-threaded operations).
- `PanicInfo::message()` (returns `fmt::Arguments`; `prelude.rs` reads `location()` instead).

## How unsupported constructs are reported

Unsupported operations fail in two explicit ways:

1. **Reachability-gated zombies (link time).** When the backend cannot lower an expression, it emits `__rt.js_abort(...)` as a placeholder and records a zombie on the item. `jsc-link` reports an error only if the item is reachable from an entry point, printing the full call chain:

```text
error: core/src/ascii/ascii_char.rs:1186:23: rustc_codegen_js cannot cast a pointer to
       `[ascii::ascii_char::AsciiChar]` into a pointer to `str`
  = note: in `<escape::EscapeIterInner<N, escape::MaybeEscaped> as fmt::Display>::fmt`
  = note: required by `<char as fmt::Debug>::fmt`
  = note: required by `str::slice_error_fail_rt`
  = note: required by `str::slice_error_fail`
  = note: required by `<core::str::pattern::StrSearcher<'a, 'b> as core::str::pattern::Searcher<'a>>::next`
  = note: required by `<&'b str as core::str::pattern::Pattern>::is_contained_in`
  = note: required by `core::str::<impl str>::contains`
  = note: required by `rust_entry`
```

2. **Runtime guards.** Checks that depend on runtime buffer metadata—such as whether a buffer holds elements of the requested size or whether a byte count is an exact multiple of the element size—live in `__rt` and throw when violated. These cases are listed in `CONTRACT.md` ("Still not supported") and pinned by the `guard_*` tests.
