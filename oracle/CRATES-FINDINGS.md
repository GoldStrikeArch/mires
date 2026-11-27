# Compiler Bugs & Limits Discovered by the Differential Oracle

Every entry below was discovered by compiling and differentially testing real
Rust code (`examples/`, `library/coretests`, `library/alloctests`, `rustc-ui`
patterns, and popular `crates.io` crates) via `cargo mires test --diff` on
pinned nightly `nightly-2026-07-27`.

---

## 1. High-Severity Runtime / Correctness Bugs

### 1.1 `#[js_export]` breaks Rust-to-Rust callers (`examples/npm-markdown`)

- **Severity**: High (breaks `cargo mires test` on any crate whose unit tests call a `#[js_export]` function directly).
- **Where observed**: `examples/npm-markdown/src/lib.rs` (`render_markdown`).
- **Symptom**: Running `cargo mires test --manifest-path examples/npm-markdown/Cargo.toml` fails all 4 unit tests with:
  ```text
  runtime error: rustc_codegen_js: entered unreachable code
  ```
- **Root cause**: `#[js_export(name = "renderMarkdown")]` emits JS boundary marshaling (`options == null ? ...`, unwrapping/throwing `Result`) directly inside the compiled Rust function body instead of generating a separate JS export wrapper around an unmodified Rust function. When Rust unit tests call `render_markdown(input, None).unwrap()`, the Rust `Option::None` representation (`{ TAG: "None" }`) is treated as non-null JS input and the unwrapped JS object return value breaks `Result::unwrap()`.
- **Minimal repro**:
  ```rust
  use mires::js_export;

  #[js_export(name = "greet")]
  pub fn greet(name: Option<String>) -> Result<String, String> {
      Ok(name.unwrap_or_else(|| "world".into()))
  }

  #[test]
  fn call_from_rust() {
      assert_eq!(greet(None).unwrap(), "world");
  }
  ```

### 1.2 `BinaryHeap::pop()` silent wrong answer

- **Severity**: Critical (silent miscompile — produces wrong ordering without trapping).
- **Where observed**: `oracle/rustc-ui` development (`std::collections::BinaryHeap`).
- **Minimal repro**:
  ```rust
  use std::collections::BinaryHeap;

  #[test]
  fn binary_heap_max_order() {
      let mut h = BinaryHeap::from([4, 1, 7, 3, 9, 2]);
      let mut out = Vec::new();
      while let Some(x) = h.pop() {
          out.push(x);
      }
      assert_eq!(out, vec![9, 7, 4, 3, 2, 1]);
  }
  ```
- **Observed behavior**: Native `rustc` produces `[9, 7, 4, 3, 2, 1]`; Mires yields elements out of heap order (likely miscompilation in `BinaryHeap::sift_down` / `Hole` pointer drop).

### 1.3 `sha2` (`v0.10.9`): runtime panic inside `Sha256::digest`

- **Severity**: High (compiles cleanly, panics at runtime on Mires while passing natively).
- **Where observed**: `crates.io` probe (`sha2 = "0.10"`).
- **Minimal repro**:
  ```rust
  use sha2::{Digest, Sha256};

  #[test]
  fn sha256_abc() {
      let h = Sha256::digest(b"abc");
      let got: String = h.iter().map(|b| format!("{b:02x}")).collect();
      assert_eq!(
          got,
          "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
      );
  }
  ```
- **Observed behavior**: Panics inside `Sha256::digest(b"abc")` before returning (core panic without formatting in `block-buffer` / `sha2::sha256::soft::compress`).

### 1.4 `bytes` (`v1.12.2`): runtime panic on `Bytes::drop`

- **Severity**: High (compiles cleanly, all buffer reads/writes produce exact right bytes, then panics on drop).
- **Where observed**: `crates.io` probe (`bytes = "1"`).
- **Minimal repro**:
  ```rust
  use bytes::{Buf, BufMut, BytesMut};

  #[test]
  fn bytes_freeze_drop() {
      let mut b = BytesMut::with_capacity(16);
      b.put_u32(0x1234_5678);
      b.put_slice(b"hi");
      let mut f = b.freeze();
      assert_eq!(f.get_u32(), 0x1234_5678);
      assert_eq!(&f[..], b"hi");
      // Panics here when `f` is dropped!
  }
  ```
- **Observed behavior**: Prints exact expected values through `assert_eq!(&f[..], b"hi")`, then panics when `f: Bytes` drops (likely custom vtable / tagged-pointer `Shared` refcount handling in `bytes::bytes::Vtable`).

### 1.5 `Box<dyn Any>` over `Vec<u8>` deallocation refusal on drop

- **Severity**: High (runtime abort on drop of valid `Box<dyn Any>`).
- **Where observed**: `oracle/rustc-ui` development.
- **Minimal repro**:
  ```rust
  use std::any::Any;

  #[test]
  fn box_dyn_any_vec_drop() {
      let _b: Box<dyn Any> = Box::new(vec![1_u8, 2, 3]);
  }
  ```
- **Error**:
  ```text
  runtime error: rustc_codegen_js: this pointer does not name a live heap block, so it cannot be deallocated
  ```
- **Note**: `Box<dyn Any>` over `i32`, `String`, and user structs drops cleanly; `Vec<u8>` specifically hits the native-Vec representation mismatch through the erased `dyn Any` drop glue.

### 1.6 `str::rsplit_once` / reverse pattern searcher runtime refusal

- **Severity**: Medium-High (runtime abort on standard `str` method).
- **Where observed**: `oracle/rustc-ui` development, `coretests::pattern`, `alloctests::string::test_replace_last`.
- **Minimal repro**:
  ```rust
  #[test]
  fn rsplit_once_char() {
      assert_eq!("abc_def_ghi".rsplit_once('_'), Some(("abc_def", "ghi")));
  }
  ```
- **Error**:
  ```text
  runtime error: rustc_codegen_js: this pointer is not a byte view of 8-byte elements, so it cannot be cast back to one
  ```
- **Root cause**: Reverse `memrchr` / `TwoWaySearcher` casts a byte slice pointer to a `usize`-aligned word view, which the typed JS memory model rejects.

### 1.7 `std::panic::catch_unwind` cannot catch `#[rustc_nounwind]` core panics

- **Severity**: Medium (affects code calling `catch_unwind` around bounds checks, arithmetic overflow, or `core::panicking::*`).
- **Where observed**: `coretests::num::int_sqrt::i*::isqrt*`.
- **Root cause**: `patches/unwind/0002-stdlib-cold-nounwind.patch` marks `core::panicking::*` functions `#[rustc_nounwind]`, so `rust_begin_unwind` rethrows them wrapped in `__rt.abort_error($e)`, which `__rt.js_catch_unwind` intentionally refuses to catch. (`mires-test` handles top-level `#[should_panic]` tests safely via `host_try_catch_all`, but user code calling `std::panic::catch_unwind` directly still aborts.)

### 1.8 `vec![...]` non-const regression from `0023-native-vec.patch`

- **Severity**: Medium (breaks `const` contexts constructing `Vec` in upstream `alloctests::vec`).
- **Where observed**: `alloctests::vec`.
- **Error**:
  ```text
  error[E0015]: cannot call non-const function `std::vec::rcgjs_vec_from_array::<u32, 3>` in constants
  ```

### 1.9 `release-balanced` (`opt-level=3`) JIR encoding failure on `AlignmentEnum` (`coretests::alloc`)

- **Severity**: Medium (passes in `dev`, fails to compile under `--release` / `-C opt-level=3`).
- **Where observed**: `coretests::alloc::layout_errors`.
- **Error**:
  ```text
  error: src/alloc.rs:59:1: 59:19: JIR encoding declined: block 14 statement 1: could not read the variant of `std::mem::alignment::AlignmentEnum` out of a constant
  ```
- **Root cause**: At `opt-level=3`, rustc constant-folds `Layout::from_size_align` into a constant `core::mem::alignment::AlignmentEnum` whose niche/repr variant cannot be decoded by Mires's JIR constant encoder.

### 1.10 `jsc-link` `release-balanced` optimizer blowup on `fmt` suites (`alloctests::fmt`, `coretests::fmt::num`)

- **Severity**: Medium (performance / scalability — blocks `--release` compilation of tiny 8–10 test formatting modules).
- **Where observed**: `alloctests::fmt` (8 tests) and `coretests::fmt::num` (10 tests) link in <1s in `dev`, but exceed 45s inside `jsc-link` under `js-profile=release-balanced`.
- **Impact**: `coretests::alloc`, `coretests::fmt::num`, `alloctests::fmt`, and large monolithic `str`/`num`/`iter` shards are covered in `dev` and explicitly listed in `release_skipped_modules` while ~930+ tests across `rustc-ui`, `coretests`, `alloctests`, and all 16 `crates.io` crates run in `release-balanced`.

---

## 2. Compile-Time Lowering Limits Observed in `crates.io` Candidates

These popular pure-Rust crates were probed and hit explicit compile-time
lowering diagnostics (no silent miscompiles):

| Crate | Version | Compile-Time Diagnostic |
|---|---|---|
| `semver` | `1.0` | `core/src/ptr/const_ptr.rs:49:9: rustc_codegen_js cannot cast a pointer to identifier::Identifier into a pointer to std::num::NonZero<u64>` |
| `url` | `2.5` | `JIR lowering of core$option$impl_0_unwrap_or_else failed: a call inside a #[track_caller] body ...` |
| `regex-lite` | `0.1` | `core/src/ptr/mod.rs:823:24: a struct with a str tail is not supported by rustc_codegen_js; only a slice tail is` |
| `toml` | `1.1` | `JIR lowering of toml_parser$lexer$lex_atom$closure_0 failed: a slice constant whose pointee is not a slice` |
| `itertools::permutations` | `0.14` | `core/src/slice/rotate.rs:52:15: rustc_codegen_js cannot cast a pointer to MaybeUninit<([usize; 32], [usize; 0])> into a pointer to usize` (note: all other `itertools` adapters pass 100% differentially!) |

---

## 3. Recurring Compiler Lowering Limits Across `coretests` & `alloctests`

| Pattern / Stdlib API | Diagnostic | Impacted Suites |
|---|---|---|
| `BTreeMap` / `BTreeSet` drop | `JIR lowering of alloc::collections::btree::node::Handle::drop_key_val: address of a direct value that is not in a cell` | `coretests::fmt::builders`, `alloctests::{autotraits, btree_set_hash, collections::eq_diff_len}` |
| `[T]::rotate_left` / `rotate_right` | `core/src/slice/rotate.rs:52: cannot cast MaybeUninit<([usize; 32], [T; 0])> pointer to *mut T` | `alloctests::vec_deque`, `itertools::permutations` |
| `dyn` trait upcasting / raw `dyn` metadata | `a dyn upcast, which needs the supertrait vtable slot` / `raw dyn pointer built from parts` | `coretests::{ptr, iter::traits::iterator}`, `alloctests::thin_box` |
| Nested array equality `[[T; N]; M] == ...` | `core/src/array/equality.rs:157` `raw_eq` zombie | `rustc-ui` |
| `alloc::rc::Rc` unsized slice allocation | `raw pointer to an unsized type` in `Rc::allocate_for_ptr_in` | `alloctests::{rc, c_str2}` |
| `alloc::boxed::Box` transmute in `LinkedList` | `alloc/src/boxed.rs:1233:18: transmute between these types is not supported` | `alloctests::linked_list` |
| `core::net::IpAddr` transmute | `core/src/net/ip_addr.rs:1499:49: transmute between these types is not supported` | `coretests::net` |
| Large array constants | `JIR encoding declined: array constant exceeds the encoded-constant length limit` | `coretests::iter::adapters::array_chunks` |
| Overaligned heap allocations (`> 16B`) | Runtime allocation alignment assertion fails | `alloctests::heap` |
