# Compiler Bugs & Limits Discovered by the Differential Oracle

Every entry below was discovered by compiling and differentially testing real
Rust code (`examples/`, `library/coretests`, `library/alloctests`, `rustc-ui`
patterns, and popular `crates.io` crates) via `cargo mires test --diff` on
pinned nightly `nightly-2026-07-27`.

---

## 1. Resolved in the Architectural Track (Seams 1–5)

All 9 high/medium-severity runtime and compile-time bugs from the initial oracle
audit — plus 6 major recurring stdlib lowering blockers — have been structurally
eliminated and are now continuously gated across `core-tests`, `metamorphic-test.sh`,
`rustc-ui`, `coretests`, `alloctests`, and `oracle/crates/`:

| Previously Open Issue | Architectural Fix | Now Continuously Verified In |
|---|---|---|
| `#[js_export]` broke Rust-to-Rust callers (`examples/npm-markdown`) | **Seam 1**: External JS export facades separated from pure internal Rust bodies (`export_boundary.rs`, `link.rs`) | `examples/npm-markdown` (`cargo mires test --diff` in CI, dev & release) |
| `BinaryHeap::pop()` silent wrong ordering | **Seam 2/Runtime**: String-keyed field slots (`$o: "elt"`) handled directly in `_run`/`copy`/`copy_nonoverlapping`/`write_bytes`/`compare_bytes` | `423_binary_heap_pop_and_field_copy`, `rustc-ui` |
| `bytes` (`v1.12.2`) runtime panic on `Bytes::drop` | **Seam 4**: Transparent-union indirectness (`place.rs`/`intrinsics.rs`), `_put` `_boxes` registration, fixed-point `typed_heap_enabled` | `oracle/crates` (`bytes`, dev & release) |
| `Box<dyn Any>` over `Vec<u8>` drop refusal | **Seam 4** + `$`-prefixed slot keys: unified `Box<dyn Trait>` / `NativeVec` carrier & drop contract | `424_seam4_box_dyn_dealloc_const_vec`, `rustc-ui` |
| `str::rsplit_once` / reverse pattern searcher abort | **Seam 3**: Scalar reverse `memrchr` in `patches/0005-scalar-memchr.patch` | `443_scalar_memrchr_rsplit`, `coretests::pattern`, `alloctests::{str,string}`, `rustc-ui` |
| `std::panic::catch_unwind` couldn't catch `#[rustc_nounwind]` core panics | **Seam 4**: Cross-realm `WeakSet` distinguishing catchable `js_panic_error` panics from compiler safety aborts + `prune_invoke.rs` landing-pad preservation | `425_catch_unwind_core_panics`, `coretests::num::int_sqrt` (20/20), `rustc-ui` |
| `vec![...]` non-const regression (`E0015`) | **Seam 4**: `const fn rcgjs_vec_from_array` in `patches/0023-native-vec.patch` | `424_seam4_box_dyn_dealloc_const_vec` |
| `AlignmentEnum` `-C opt-level=3` JIR constant encoding failure | **Seam 5**: Raw discriminant preservation for `EnumRepr::Number` in `backend/src/jir/encode/constant.rs` | `434_const_decoding_completeness`, `coretests::alloc` (release-balanced) |
| `BTreeMap` / `BTreeSet` drop & `MaybeUninit::spec_fill` (`address of direct value not in cell`) | **Seam 2**: Unified addressable-place rule (`ensure_addressable_cells` in `ssa.rs`) + ZST deref/field projection (`place.rs`) | `422_seam2_btreemap_maybe_uninit_zst_box`, `coretests::fmt::builders`, `alloctests::{autotraits,btree_set_hash,boxed}` |
| `[T]::rotate_left`/`rotate_right` (`MaybeUninit<([usize; 32], [T; 0])>`) | **Seam 3**: `__rt.uninit_cast` lazy scratch buffers in `ptr.rs` + `runtime/shim.js` | `441_slice_rotate_vecdeque`, `alloctests::{slice,vec_deque}`, `itertools::permutations` |
| `dyn` trait upcasting, `ptr::from_raw_parts`, const-generic vtable collisions | **Seam 5**: `JIR_SCHEMA = 25` (`CastKind::DynUpcast { slot }`, `DynMetadata` transparency, `pointer_from_parts`, const-generic + `type_id_hash` vtable naming in `naming.rs`) | `431_dyn_trait_upcast`, `432_ptr_from_raw_parts_and_rc_unsized`, `coretests::iter::traits::iterator` (42/42), `rustc-ui` |
| `url` (`#[track_caller]` ABI mismatch) | **Seam 5**: Explicit `SourceFactId::{FORWARD, NOT_TRACKED}` sentinels (`JIR_SCHEMA = 25`) | `430_track_caller_explicit_abi`, `oracle/crates` (`url`, dev & release) |
| `regex-lite` & `Rc<str>` / `Arc<str>` (`str`-tail DSTs) | **Seam 5**: Generalized `UnsizedTail` (`place.rs`/`ptr.rs`/`intrinsics.rs::size_of_tail`) + direct `[u8]`-tail → `str`-tail cast in `0022-typed-rc-slice.patch` | `433_str_tail_dst`, `oracle/crates` (`regex-lite`, dev & release), `rustc-ui` |
| `core::net::Ipv6Addr`, `NonZero<char>`, `LinkedList` `Box` transmute, nested array `raw_eq` | **Seam 3**: Layout-driven `pod_codec.rs` (`pod_transmute`, `emit_pod_raw_eq`, niche codecs), structural aggregate transmute, offset-0 prefix casts, `ArrayBuffer`/`TypedArray` `alloc_zeroed` & views | `440`–`446`, `coretests::{net,num::niche_types}`, `alloctests::linked_list`, `rustc-ui` |
| `coretests::{ffi, option, iter::adapters::array_chunks}`, `waker`, `alloctests::task` | **Seam 5** + integration fixes: `[i8] <-> [u8]` slice `byte_sign`, CTFE `ClosureOnceShim` wrapper, shared hoisted immutable `ConstTree::Reference` (`$cr$h...`), ZST large array/slice constants | `coretests::{ffi,option,waker,iter::adapters::array_chunks}`, `alloctests::task` |

---

## 2. Remaining Open Issues

### 2.1 `jsc-link` `release-balanced` optimizer latency on `fmt` / monolithic numeric & `str` suites

- **Severity**: Medium (scalability — compile-time latency in `--release`, not correctness; all tests pass in `dev`).
- **Where observed**: `alloctests::fmt` (8 tests), `coretests::fmt::num` (10 tests), and monolithic `coretests::num` / `coretests::iter::adapters` / `alloctests::str` shards link in <1s in `dev`, but exceed the 45s per-shard budget inside `jsc-link` under `js-profile=release-balanced`.
- **Impact**: Covered in `dev` and tracked in `release_skipped_modules` while **1,295 tests** run in `release-balanced` across `rustc-ui`, `coretests`, `alloctests`, and all 19 `crates.io` crates.

### 2.2 Remaining `crates.io` Candidate Findings

| Crate | Version | Status / Root Cause |
|---|---|---|
| `sha2` | `0.10` | Compiles cleanly; at runtime `generic_array::GenericArray<u8, U64>` (64-byte nested `#[repr(C)]` struct of `u8`s) is cast via `*mut GenericArray<u8, U64> -> *mut u8` for `slice::from_raw_parts_mut`, hitting Mires's fail-closed non-scalar byte-slice guard (`cannot view 64-byte non-scalar elements as a byte slice`). |
| `semver` | `1.0` | Compiles cleanly via `pod_view`; at runtime `semver::Identifier` (`#[repr(C)]` 8-byte union/struct) still hits `cannot view 8-byte non-scalar elements as a byte slice` when inspecting raw identifier bytes. |
| `toml` | `0.8` / `1.2` | Compiles cleanly (`Arc<str>` + `ConstTree::Slice` fixed); at runtime `winnow::stream::Offset::offset_from` computes token spans by subtracting raw addresses of `&str` subslices (`sub.as_ptr() as usize - base.as_ptr() as usize`), which does not hold when JS string primitives materialize fresh byte buffers on `.as_ptr()`. |

### 2.3 Remaining Localized `coretests` & `alloctests` Limits

| Pattern / Stdlib API | Diagnostic / Root Cause | Impacted Suites |
|---|---|---|
| Vtable pointer constants (`GlobalAlloc::VTable`) | `JIR encoding declined: pointer to a vtable constant is not encoded` | `coretests::{mem::trait_info_of, ptr}`, `alloctests::thin_box` |
| Static sub-slice constants & non-trivial `align_to` | `U64 -> U64U64U32` pointer cast / partial `static` slice constant | `coretests::slice` |
| `Vec::into_iter` ZST transmute (`vec.rs:2020-2024`) | `transmute` between `alloc::vec::IntoIter` layout variants | `alloctests::vec` |
| `Rc::<str>::from_raw` & custom non-slice/str/dyn tails | Compile-time refusal (`guard_rc_str`) because a bare `*const str` carries no enclosing `RcInner` block | `alloctests::rc` |
| Overaligned heap allocations (`> 16B`) | Runtime alignment check on JS heap arrays | `alloctests::heap` |
| External `C` `strlen` / `std::os` / `std::sync::{mpsc, UniqueArc, ReentrantLock}` / `rand` | Host/OS/dev-dep APIs outside `mires-std` | `coretests::{array, atomic, error, simd, intrinsics}`, `alloctests::{arc, c_str, c_str2, collections::binary_heap, sort, sync}` |
| Localized runtime/unwind edge cases | `any::dyn_type_name`, `future::test_join`, `hint::select_unpredictable_drop_on_panic`, `iter::adapters::map_windows::output_n2`, `option::as_slice`, `string::test_retain`, plus localized `boxed` (3), `slice` (3+2 panic-fmt), and `vec_deque` (9) unwind/drop tests | Tracked individually in `coretests/skiplist.json` & `alloctests/skiplist.json` |
