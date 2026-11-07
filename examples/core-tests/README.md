# The `core` test suite

These are ordinary `#![no_std]` crates. There is no `mini_core`, no `#![no_core]` and no hand
written lang item: every one of them is compiled against the **real `library/core`**, itself built
by this backend.

```
./scripts/build_sysroot.sh      # ~1 minute, once
./scripts/test.sh               # runs examples/tests/ and then this suite
./scripts/test.sh 04_iter_range # one test
```

`scripts/build_sysroot.sh` copies `rust-src` into `build/stdlib/`, applies `patches/`, compiles
`core` and `compiler_builtins` with `-Zcodegen-backend`, and installs the two rlibs into
`build/sysroot/`. `scripts/compile_core.sh` then compiles a test with `--sysroot build/sysroot
--crate-type cdylib`, which routes through `CodegenBackend::link` — whole-program dead code
elimination, and a zombie report for anything reachable that the backend cannot express.

Each test has a `.expected` (exact stdout), a `.maxbytes` (a size budget, to catch a lost DCE) and,
for `10_panic` and every `guard_*` test, an `.expect_abort` marker meaning the program must exit
non-zero. Expectations were validated by transplanting each test onto `std` and running it
natively; the only deliberate divergences are the two places where the answer depends on the
pointer width (see `06_num`'s `usize::BITS` and `07_mem`'s `size_of::<&i32>()`, which are 32 and 4
on `wasm32-unknown-unknown` and 64 and 8 on a 64 bit host).

## What is in

| test | covers |
|---|---|
| `01_option` | predicates, `unwrap`/`unwrap_or`/`unwrap_or_else`/`unwrap_or_default`, `map`/`and_then`/`filter`/`or`/`map_or`, `?`, `take`/`replace`/`as_mut`/`flatten`, `PartialEq`, the `Option<&T>` and `Option<bool>` niches |
| `02_result` | `is_ok`/`is_err`, `unwrap_or*`, `map`/`map_err`/`and_then`, `ok`/`err`, `?` through a custom error enum, `Result<T, ()>` |
| `03_cmp` | `Ordering` on every primitive, `reverse`/`then`/`is_lt`, `min`/`max`/`clamp`, derived `PartialOrd`/`Ord` on a struct and an enum, a hand written `Ord`, a generic function over `Ord` |
| `04_iter_range` | `for`, `..=`, `step_by`, `rev`, `sum`/`product`/`count`/`fold`/`max`/`min`/`last`/`nth`, `map`/`filter`/`skip`/`take`/`take_while`/`skip_while`/`filter_map`/`chain`/`zip`/`enumerate`, `any`/`all`/`find`/`position`, `next` by hand, a `u64` range, `Range` as a value |
| `05_slice_index` | array → slice coercion, `len`/`is_empty`, indexing (constant and computed), `get`/`get_mut`/`first`/`last`/`last_mut`, mutation through `&mut [T]`, slice patterns including `rest @ ..`, arrays of structs, 2-D arrays |
| `06_num` | the type constants, `wrapping_*`/`checked_*`/`overflowing_*`/`saturating_*`, `count_ones`/`leading_zeros`/`swap_bytes`/`rotate_left`, `abs`/`signum`/`pow`/`next_power_of_two`, `div_euclid`/`rem_euclid`, the whole cast matrix across the number/`BigInt` boundary, saturating float→int, `NonZero::new` |
| `07_mem` | `size_of`/`align_of`, `replace`/`take`/`swap`/`drop`/`forget`/`ManuallyDrop`, `Drop` order for locals, fields, early returns and moves |
| `08_dyn` | `&dyn`/`&mut dyn`, overridden and default methods, a supertrait, `dyn` in a struct field and in an array, `&mut dyn FnMut` |
| `09_closure` | `Fn`/`FnMut`/`FnOnce`, captures by reference and by value, `impl Fn` returns, `fn` pointers from items and from non-capturing closures, closures through `core`'s combinators |
| `10_panic` | `Option::unwrap` on `None` → `core::panicking::panic` → this crate's `#[panic_handler]`, with the `#[track_caller]` `Location` pointing at the caller, then a non-zero exit |
| `11_fmt` | `write!` against the real `core::fmt`: the format string template, `Display` for the integers, `str` and `char`, width, fill and radix in `Formatter`, explicit argument indices, and `Arguments::as_str` |
| `12_slice_iter` | `iter()`/`iter_mut()`/`for x in &xs`, `rev`/`nth`/`position`/`find`/`enumerate`/`zip`/`map().sum()`/`last`/`min`/`max`/`fold`, `len`/`as_slice` on a partly consumed iterator, `split_at`, subslices, and elements that are structs or zero sized |
| `13_sort` | `sort_unstable`/`sort_unstable_by`/`sort_unstable_by_key` on `[i32]`, `[u64]` and a struct at lengths 0, 1, 2, 16 and 40, plus `swap`, `reverse`, `contains`, `binary_search`, `copy_from_slice`, `starts_with`/`ends_with`, `fill` |
| `14_str` | `len`/`is_empty` on ASCII and non-ASCII, `as_bytes` and indexing it, `bytes()`, `chars()`, `char_indices()`, `==`/`!=`/`<`/`cmp`, `starts_with`/`ends_with`, `contains`, `find(char)` and `find(&str)`, `&s[a..b]` and `get(a..b)`, `trim`, `split(char)`, `from_utf8`, `escape_ascii`/`escape_debug`, `char::len_utf8` |
| `15_ptr` | `null`/`is_null`, `without_provenance` and the `addr` round trip, `ptr::eq` of two raws to one object, `offset`/`add`/`sub`/`offset_from`, `read`/`write`, `read_unaligned`/`write_unaligned`, `byte_add`/`byte_offset`/`byte_sub` over byte and non-byte pointees, `copy` overlapping both ways, `copy_nonoverlapping`, `write_bytes`, `swap`, `&raw` of a struct field, the `*const T` → `*const u8` chain, `from_le_bytes`/`to_le_bytes` at 32 and 64 bits, and `[u8; N]`/`[i32; N]` equality |
| `19_float_fmt` | `Display`/`LowerExp` for `f32`/`f64`: precision, width, fill, sign, the `f64::MAX`/`f64::MIN_POSITIVE` long forms, `NaN`/`inf`/`-0`, `{:?}` on a `char`, and `str::parse::<f64>()` back |
| `20_alloc_box` | `Box` in the four shapes it comes in -- a scalar, a struct written through, a trait object and a boxed slice -- and the drop that runs the pointee's glue and frees the block |
| `21_vec` | `Vec` over the retype: `push` growth and the in-place `realloc`, the zeroed block `vec![0; n]` asks for, the borrow as an ordinary `&[T]`, with both a scalar element and an aggregate one |
| `22_string` | `String`, which is a `Vec<u8>` and nothing else: building one byte by byte and piece by piece, and the decode to `&str` that only happens on a deref |
| `23_format` | `format!`, which is `core::fmt` writing into a heap block that grows under it: width, fill and precision against an allocating sink, plus the host string builder `prelude::JsStr` wraps |
| `24_enum_repr` | `core`'s own enums at every representation: `Ordering` and `mem::Alignment` fieldless with an integer `repr`, `Option<T>`'s niches for a `&T`, a `NonZero` or a `char`, the whole-value writes `take`/`replace`/`insert` make through a `&mut Option<T>`, and `mem::discriminant` |
| `25_array_ref` | `&*p` where the pointee is `[E; N]`, from a plain slot, from the window an `as *mut [E; N]` cast produces and from a slice record -- `itoa::Buffer`'s shape, which is why `serde_json` could parse integers but not print them |
| `26_async` | the coroutine state machine as an enum: the state index as tag, saved locals keyed by `CoroutineSavedLocal` so one survives a suspension, construction and state change. No executor; `block_on` polls by hand |
| `27_dangling` | the pointer a collection holds before it has ever allocated: `NonNull::dangling()` built into a slice, so a `for` over a `Vec` that has never been pushed to runs |
| `28_array_into_iter` | `for x in arr` by value: `array::IntoIter` through `PolymorphicIter`'s unsized tail, so a pointer to a struct with a trailing slice is `{ ptr, meta }`, from both alive ends and with a `Drop` element |
| `29_rc` | `Rc` and `Arc`, both halves. Sized: the strong and weak counts, `get_mut` gated on uniqueness, `ptr_eq` on one block and on two, a `Weak` that does not keep its value alive, the payload glue that runs when the LAST owner goes, `Rc<RefCell<T>>` written through a clone. Unsized: `Rc<[u8]>`, `Rc<[u32]>`, `Rc<[String]>` through the `from_iter` path, an element with a `Drop`, `Arc<[T]>` and an empty tail |
| `30_hashmap` | `mires_js::collections::{HashMap, HashSet}`: every key class including the BigInt widths and the `str` class, `insert` returning the old value, a struct value written through `get_mut`, `remove`, snapshot iteration, and set dedup |
| `guard_as_chunks` | a gap that is a run time guard rather than a zombie: `as_chunks` aborts the program instead of answering |
| `guard_heap_retype` | reading one heap block at two element sizes once something has been written at the first: `__rt.unscale` throws rather than hand back a wrong number |
| `guard_map_key` | a gap that is a **type** error: an aggregate cannot be a host `Map` key, and the sealed `MapKey` trait says so at the call site |
| `guard_rc_str` | the boundary of the unsized heap tail: a slice tail is a run of elements the block can hold and a `str` tail is not, so `Rc<str>` is reported rather than answered |
| `guard_use_after_free` | a cast through a pointer into a block that has been freed: `dealloc` poisons the block and remembers it, and the next cast says so |
| `guard_zeroed_retype` | a zeroed block retyped to something whose zero has no spelling here: a fat pointer, because there is no buffer for an all-zero `&[T]` to name |

`30_hashmap` and `guard_map_key` are the only tests that link a crate beyond `core` and `alloc`.
`scripts/test.sh` builds `mires-js` through the backend into an rlib and offers it to every test
with `--extern`; a test that never names it links nothing. `std`'s `HashMap` is out for the reason
`CONTRACT.md`'s "Host collections" gives: `hashbrown` reads one allocation at two element
granularities, which is the refusal `guard_heap_retype` pins. The host has a hash table, and that is
the one these use.

`12_slice_iter`, `14_str` and `15_ptr` are the stage 1.5 suite. Before the `{buf, off}` pointer
model all three were impossible: `[T]::iter` is a pair of raw pointers walked with `add` and
subtracted with `offset_from`, `str` is a byte buffer as soon as `core` touches it, and
`core::ptr` is the thing itself. A pointer is now a **slot** — a JavaScript container plus a key
into it, with the offset counted in elements — so a dereference is an assignable place, an offset
is a new record, and all of it has an exact JavaScript form. `CONTRACT.md` has the model;
`15_ptr` is its unit test.

The regression that section exists for is `s.as_bytes().len()`. It used to return `undefined`: a
`*const u8` taken from a string was the string itself, and the slice built from it had no length.
It was the only silent miscompile the spike ever had, and `14_str` pins it.

## What is out, and why

Everything below has one cause or the other. They are not arbitrary gaps; each is a consequence of
a documented decision or of a specific unfinished piece, and each has a specific thing that would
remove it.

### 1. Reinterpreting a buffer at another granularity

A slot can re-key a buffer, but it cannot invent elements. Reading a `[i32]` as `[[i32; 2]]`, or as
the `usize`s the machine would see, asks for elements that are not there — and nothing in the
*type* says whether a pointer to `[E; N]` is a real slot or a window an earlier cast produced over
somebody else's buffer. So the question is asked when the program runs, and `__rt.chunk_slice` or
`__rt.unscale` throws:

* `as_chunks`, `array_chunks`, `chunks_exact`, `align_to`. `guard_as_chunks` pins the abort.
* through `align_to`, `memchr`'s word-at-a-time path: `str::find(char)` and `split(char)` answer
  while the haystack is shorter than two `usize`s and throw past that, and `rfind` (`memrchr`)
  never answers. An index loop over `as_bytes()` is the substitute, and `14_str` uses one.
  `find(&str)` and `contains` are *not* subject to this: they run the two way `StrSearcher`, which
  compares slices rather than reading words, and `14_str` covers them at every length.
* a pointer cast between two pointees that are neither the same size nor a byte. This one *is* a
  zombie, reported at the cast with both types named; `examples/tests/zombie_reachable.rs` pins it.

`ptr_mask` is out permanently and for a different reason: it asks for the bits of an address, and
an address here is a synthetic number rather than something a buffer is stored at.

### 2. Formatting, past the part `11_fmt` covers

`write!` works, and so do the `Display` impls behind it — including the **floats**, which `19_float_fmt`
covers end to end against a native transplant: the full `Display`/`LowerExp` surface, `f64::MAX` and
`f64::MIN_POSITIVE` printed without an exponent, the non-finite spellings, and `str::parse::<f64>()`
back the other way.

What is still out is `Debug` **as a bound**:

* `#[derive(Debug)]`, and `Result::unwrap`/`Result::expect`, whose `E: Debug` bound coerces to a
  `&dyn Debug` and drags every `Debug` impl in the program into reachability. Use `.ok().unwrap()`,
  `unwrap_or*` or a `match`.

`{:?}` on a concrete type is *in* — `19_float_fmt` formats `char`s with it. That is what unblocked
the rest of this section: `char`'s `Debug` escapes through a buffer of `AsciiChar` and calls
`as_str` on it, and a slice of any one-byte element now decodes to the string its bytes spell
(`CONTRACT.md`, "`str` is a hybrid"). `AsciiChar` is a fieldless one-byte enum, so it is
`{ $t: n }` here and the decoder reads the tag; `examples/tests/16_byte_enum_str.rs` pins that cast
at mini_core scale, where it is one line rather than a dozen frames down inside `core`.

So **`&s[a..b]`, `contains` and `find` with a `&str` pattern all work now**, and `14_str` exercises
them. All three reach `str::slice_error_fail`, which is why they were out: slicing can panic, the
panic names the offending `char`, and naming a `char` needed that cast. `str::get(a..b)` is still
there too, because the `None` cases have no panicking spelling.

`patches/0001-panic-without-formatting.patch` is still applied, and still does exactly what it did:
the runtime arm of `core`'s `const_panic!`, `panic_bounds_check`, `panic_display` and
`assert_failed` panic with a fixed string rather than a formatted one, so bounds-checked indexing,
`clamp`, `Option::expect`, `assert!` and `unreachable!` all work with a less specific message.

### 3. Unfinished, rather than decided

These have a known cause and no design reason to stay:

* `NonNull::dangling()`, which is `transmute::<Alignment, NonNull<T>>`. `Alignment` now converts to
  an *integer* (it is a fieldless enum with a direct tag, so its value is its discriminant), but the
  pointer direction and the `Option<NonZero<usize>>` niche that `NonNull::addr` needs are still
  missing. `15_ptr` builds the same pointer with `without_provenance(align_of::<T>())` instead.
* `Ipv6Addr`, which parses and prints through a `[u16; 8]`/`[u8; 16]` reinterpretation — the
  boundary in section 1, reached by a type rather than by a slice method.

#### The zombie census

The count is measured rather than remembered: the item tables are decoded out of the `core` and
`alloc` rlibs the sysroot was built from, which is where a zombie is recorded. It lives in exactly
one place, `CONTRACT.md`'s "The zombie census in `core` and `alloc`", which tables every survivor
with its group and the reason it stays. This page does not restate those numbers, so it cannot
drift from them; every group in that table is a boundary named on this page.

There are **no `u8 as char` zombies**: that arm is answered in `rvalue.rs` and `intrinsics.rs`.
Nothing on the census is reachable from string handling, formatting or slices.

### 4. Everything else

* an allocation whose pointee has a trailing unsized field that is **not a slice**: `Rc<str>` and
  `Arc<dyn Trait>`, and `core::ffi::CStr` and `core::wtf8::Wtf8` for the same reason. A block is
  laid out as one element type, so a header plus a run of elements fits and a header plus a
  JavaScript string does not (`guard_rc_str`). `Rc::into_raw`/`from_raw` are out with them: their
  `byte_sub` steps backwards across the header. `Rc<[T]>` and `Arc<[T]>` are in, and so are `Box`,
  `Vec`, `String` and `format!` (`20_alloc_box` through `23_format`, `29_rc`).
* writing through a `&mut str`, which has no mutable JavaScript form.
* `raw_eq` on an aggregate that is not an array of scalars, and `write_bytes` on any aggregate.
* `TypeId` equality: the backend has no stable byte representation for it.
* `f16` / `f128`: JavaScript has neither.
* SIMD: every `simd_*` intrinsic is a zombie.
* atomics with real concurrency: the atomic intrinsics lower to plain single-threaded operations.
* `PanicInfo::message()`: it is a `fmt::Arguments`, and the handler in `prelude.rs` reads
  `location()` instead, which is a plain struct.

## How a gap reports itself

Nothing here fails silently. An expression the backend cannot lower becomes a *zombie*: the item is
still emitted, with a `__rt.js_abort(...)` in place of that expression, and the link step reports it
only if the whole-program walk finds it reachable — with the chain that made it reachable:

```
error: core/src/ascii/ascii_char.rs:1186:23: rustc_codegen_mires cannot cast a pointer to
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

That is `"hello".contains("ell")`, in full: a substring search slices, slicing can panic, the panic
formats a `char`, and formatting a `char` reinterprets a buffer of `AsciiChar` as a `str`.

That chain is the reason the boundaries above are stated as precisely as they are: each one was read
straight off a report like this.

The second shape is the run time guard. A few questions cannot be answered when an item is lowered
because the type does not carry the answer — whether a buffer holds the elements a cast asks for,
whether a byte count is a whole number of them — so the check moves into `__rt` and throws. Those
are listed in `CONTRACT.md` under "Still not supported", and `guard_as_chunks` is the test that
keeps one of them loud.
