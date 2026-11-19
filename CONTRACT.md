> Recovery baseline: this document was imported from the September 7 v2 snapshot.
> It describes historical mechanisms and limits, not completed October recovery or production qualification.
> See `docs/V2-RESTORATION.md` and the current `VERDICT.md`.

# Spike interface contract

Every component (backend, mini_core, tests, shim, scripts, demo) is built by a different
agent in parallel. This file is the single source of truth for the interfaces between them.
Do not deviate from it; if something proves impossible, note it in your report instead of
silently changing the contract.

Full plan: /Users/mpl4/.claude/plans/buzzing-wondering-pillow.md
Reference cg_clif clone (read-only):
/private/tmp/claude-501/-Users-mpl4-Desktop-workspace-OSS-topcoat/61e61eec-1ccc-45c3-838d-971b5be2d33f/scratchpad/compilers/rustc_codegen_cranelift

## Toolchain

`nightly-2026-07-27` pinned by ./rust-toolchain.toml (components rust-src, rustc-dev,
llvm-tools). Being installed at spike start; may still be downloading — poll
`rustup run nightly-2026-07-27 rustc --version` before relying on it.

## Compilation command (produced .js file)

```
rustc +nightly-2026-07-27 \
  -Zcodegen-backend=<abs path to target/release/librustc_codegen_js.dylib> \
  --edition 2021 --crate-type lib --emit=obj \
  -Ccodegen-units=1 -Cpanic=abort -Coverflow-checks=off -Cdebuginfo=0 \
  -o <out>.js <test>.rs
```

The "object file" the backend writes IS the JavaScript text. If rustc mangles the output
path, scripts/compile.sh normalizes it afterward.

## Test crate shape (no_core, single crate, no linking)

Each test in examples/tests/NN_name.rs starts with EXACTLY this header:

```rust
#![feature(no_core, lang_items, intrinsics, rustc_attrs, decl_macro)]
#![feature(auto_traits, freeze_impls, unboxed_closures)]
#![allow(internal_features, dead_code, unused_variables)]
#![no_core]
#![no_main]

#[path = "../mini_core.rs"]
mod mini_core;
use mini_core::*;

#[no_mangle]
fn rust_entry() {
    // test body
}
```

`examples/mini_core.rs` contains ITEMS ONLY (no crate-level `#![...]` attributes — those
live in each test's header above; if mini_core needs more feature gates, add them to the
header spec here and use the same header everywhere).

## Print/abort shim (the ONLY foreign functions)

mini_core declares exactly these foreign items plus safe wrappers:

```rust
extern "C" {
    fn js_log_i32(x: i32);
    fn js_log_i64(x: i64);
    fn js_log_f64(x: f64);
    fn js_log_bool(x: bool);
    fn js_log_str(ptr: &str);      // backend passes the JS string value directly
    fn js_abort(msg: &str) -> !;
}
pub fn print_i32(x: i32) { unsafe { js_log_i32(x) } }
pub fn print_i64(x: i64) { unsafe { js_log_i64(x) } }
pub fn print_f64(x: f64) { unsafe { js_log_f64(x) } }
pub fn print_bool(x: bool) { unsafe { js_log_bool(x) } }
pub fn print_str(s: &str) { unsafe { js_log_str(s) } }
```

Backend lowers any call to a foreign item (no MIR) to `__rt.<symbol_name>(args...)`.

runtime/shim.js defines (plain script, NOT an ES module):

```js
globalThis.__rt = {
  js_log_i32: (x) => console.log(x),
  js_log_i64: (x) => console.log(x.toString()),
  js_log_f64: (x) => console.log(x),
  js_log_bool: (x) => console.log(x),
  js_log_str: (s) => console.log(s),
  js_abort: (msg) => { throw new Error("rust abort: " + msg); },
};
```

Number printing: js_log_f64 must print `3` for 3.0 (JS default). Tests' .expected files
must match node's default console.log formatting (booleans print as `true`/`false`).

## Emitted JS shape (backend output)

- Plain script (no ES modules, no imports). One top-level
  `function <name>(arg0, arg1, ...) { ... }` per monomorphized item.
- `#[no_mangle]` / `#[export_name]` items use the exact symbol name; everything else uses
  a sanitized unique name (def-path with `::` -> `__`, `$`-prefixed hash suffix allowed).
  Spike relaxation: falling back to the v0-mangled symbol name for internal items is
  ACCEPTABLE (nothing external depends on them); sanitized names are preferred for
  debuggability but not required for the milestone.
- Runner executes `cat runtime/shim.js <out>.js scripts/entry.js | node -`, where
  scripts/entry.js is exactly `rust_entry();`. So `rust_entry` must be a top-level
  function declaration in the emitted output.

## Value representation (backend AND tests must agree)

- i8..i32/u8..u32, isize/usize: JS numbers. i32 arithmetic masked with `|0`, u32 with
  `>>>0`, i8/i16 sign-extended via `<<24>>24` / `<<16>>16` as needed, `Math.imul` for
  32-bit multiply. usize/isize treated as safe integers (no masking).
- Wide integers (stage 1): i64/u64/i128/u128 are JS **BigInt**, not numbers. Masking is
  `BigInt.asIntN`/`BigInt.asUintN`; literals print bare via `.toString()` (never with a
  trailing `n`), which is what `js_log_i64` does. Mixed-width operations coerce at the
  boundary: a shift amount arriving as a Number must become a BigInt, and a
  BigInt<->Number cast must go through `Number()`/`BigInt()` plus the target's mask.
  Tests 01-08 stay Number-only; 12_bigint.rs is the BigInt suite.
  Related: stage 1 compiles for `wasm32-unknown-unknown`, so `usize`/`isize` are exactly
  32 bits and mask with `>>>0`/`|0`. 13_usize.rs encodes that (its expectations were
  validated natively with `usize` transplanted to `u32`).
- f32/f64: JS numbers (f32 without fround in the spike). bool: JS booleans. char: number.
- `&str`: JS string. String indexing/bytes: NOT in scope for tests.
- Structs: `{ field_name: v, ... }` (real field names). Tuples & arrays: JS arrays.
  ZSTs/unit: `undefined`.
- Enums: three shapes, chosen by the enum's own declaration -- see "Enums (stage 3)" below,
  which is the specification. In short: a fieldless `#[repr(int)]` enum is the plain number its
  discriminant is; any other fieldless enum is the bare **string** naming the variant; an enum with
  a payload-carrying variant is `{ TAG: "<VariantName>", _0: ..., _1: ... }`, with named-field
  variants using their field names alongside `TAG`.
- References: `&`/`&mut` to an aggregate is just the value (aliasing by JS object identity).
  Everything else is a `{ buf, off }` **slot**, and an address-taken local of such a type is
  boxed — see "Pointers (stage 1.5)" below, which supersedes this line. Cells (`{ v: value }`)
  and accessors (`{ get, set }`) were the stage 0 and stage 1 spellings and no longer exist.
- Copy of an aggregate = shallow clone; Move = alias.
- A closure environment is an array of its captures in MIR field order. Const propagation may
  leave that environment in immutable allocation bytes; all three compiler lanes decode those
  bytes capture by capture to the same array rather than rejecting the closure type.
- `Box<T>` is **the pointer it owns**, and nothing else: a slot for a sized `T`, a fat slice for
  `Box<[T]>`, a fat dyn for `Box<dyn Trait>`. See "Allocation".
- `String` has no representation of its own: it is a `Vec<u8>`, a heap block of one byte elements,
  and becomes a JavaScript string only where something derefs it to a `&str`.

## Test list (examples/tests/) and what each may use

01_arith.rs      ints (wrapping via ops), f64, bool, comparisons, print shims
02_control.rs    if/else, while, loop/break/continue, nested match on ints
03_structs.rs    structs, tuples, field mutation, &/&mut to locals and fields
04_enums.rs      multi-variant enum with payloads (incl. one named-field variant), match
05_recursion.rs  direct recursion (factorial/fib) + mutual recursion (is_even/is_odd)
06_generics.rs   generic fn + generic struct, >=2 instantiations
07_traits.rs     trait w/ default + overridden method, static dispatch, generic bound
08_closures.rs   Fn/FnMut closures (by-ref and by-value capture), closure passed to
                 a generic fn taking `impl Fn(i32) -> i32`-style bound (via mini_core Fn traits)

Added with the stage-1 language core, and green since:

09_drop.rs       Drop impls, drop order, nested droppable fields, early-return drops
10_dyn.rs        &dyn / &mut dyn Trait, vtables, default and overridden methods
11_statics.rs    scalar/array/struct/&str statics, `static mut` through unsafe
12_bigint.rs     i64/u64 arithmetic, div/rem truncation, shifts, casts, i64 switch
13_usize.rs      usize/isize as EXACTLY 32 bits (wrapping, indexing, casts)
14_slices.rs     &[T]/&mut [T] coercion, indexing, slice patterns, ptr_metadata length
15_intrinsics.rs the intrinsics table, at mini_core scale
16_byte_enum_str.rs  a slice of one-byte `#[repr(u8)]` enum values cast to `*const str`,
                 through both `as` and `transmute`: the mini_core-scale pin for the cast
                 that `[AsciiChar]::as_str` is in real `core`

Each NN_name.rs has NN_name.expected = exact expected stdout (newline-terminated).
Tests must be deterministic, print-based, and stay within mini_core's vocabulary.
For 01-08 that vocabulary excludes Drop impls, dyn, trait objects and slices; 09-14
are exactly the tests that add them. No std, no core, no alloc, no Vec, no string
formatting anywhere -- print numbers and literal strs only.

## Scripts

- scripts/build.sh    — cargo build --release from the spike root: the backend dylib and the
                        `jsc-link` binary it execs at the link step, printing both paths
- scripts/compile.sh  — $1=src $2=out.js; runs the rustc command above
- scripts/entry.js    — exactly `rust_entry();`
- scripts/test.sh     — build backend, then for each examples/tests/*.rs: compile,
                        run `cat runtime/shim.js out.js scripts/entry.js | node -`,
                        diff against .expected; print PASS/FAIL summary; nonzero exit on
                        any failure. Artifacts go to build/ (gitignored). Also runs the
                        examples/core-tests/ suite when a sysroot exists.
- scripts/build_sysroot.sh — patch and build the real `core`, `compiler_builtins` and `alloc`
                        with this backend into build/sysroot; run once before the core suite
- scripts/compile_core.sh — $1=src $2=out.js against that sysroot (cdylib, so the link
                        step runs)
- scripts/emit-test.sh — the emitted-JavaScript goldens (see "Emit goldens" below);
                        UPDATE_EXPECT=1 rebaselines
- scripts/module-test.sh — compiles every emit fixture as a script and as an ES module and holds
                        that the two agree item for item (see "Modules")
- scripts/run-esm.sh  — runs a `js-modules=esm` program under node: the ES module form of the
                        shim beside it, `rust_entry` imported by name
- scripts/make-esm-shim.mjs — derives that ES module form from runtime/shim.js
- scripts/check-sourcemap.js — decodes and checks a `.map` the backend wrote (see
                        "Source maps"); `node scripts/check-sourcemap.js build/03_structs.js`
- scripts/dom-test.sh — the `view!` suite: expands each examples/dom-tests fixture, compiles it as
                        an ES module, runs it against the contract's recording stub and diffs both
                        the JavaScript and the trace (see "Templates"); UPDATE_EXPECT=1 rebaselines.
                        Framework-specific: without a `view-dom-macro/` checkout it prints SKIP and
                        exits 3 (see "The suite")
- scripts/dom-trace.mjs — runs one compiled client module against contract/harness/trace.mjs and
                        prints the trace as JSON Lines

## Stage 1 + 2 deltas

Everything above describes the stage-0 spike and still holds except where this section says
otherwise. Each entry is a decision that was made while building stage 1 or stage 2 and that
another component has to agree with.

### Compilation

The command in "Compilation command" gains `--target wasm32-unknown-unknown` and
`--remap-path-prefix "$root/="` (the second so that a `#[track_caller]` location is repository
relative rather than a path to somebody's checkout — see "Hoisted constants"). Nothing is ever
handed to LLVM, so the only thing the target decides is the data layout the backend reads off
`tcx` — and a 32-bit pointer width is what makes `usize`/`isize` exactly `u32`/`i32`, so layouts,
niches, `size_of` and the constants rustc folds all agree with the JS side by construction. No
wasm toolchain component is needed: a `#![no_core]` crate compiled with `--emit=obj` never links.

Tests against the real `core` use a second command (`scripts/compile_core.sh`):
`--sysroot build/sysroot --crate-type cdylib`, the sysroot being what `scripts/build_sysroot.sh`
builds. `cdylib` is the point — it makes rustc run `CodegenBackend::link`, which is where
whole-program dead code elimination and zombie reporting happen.

Incremental rustc invocations require `-Cllvm-args=js-producer=<sha256>`, where the digest covers the selected backend library. The compile and sysroot scripts supply it automatically. The producer verifies it before queries run. A missing incremental identity, stale digest, or duplicate identity is an error. Rustc tracks the argument, so replacing a backend at the same path invalidates its query and codegen caches while an unchanged backend permits reuse. The digest is independent of linker policy and is excluded from normalized emit options. Executable cleanup MIR archives carry and verify this producer identity separately.

`js-cleanup=on` is an experimental producer option, off by default. It retains executable ordinary and coroutine cleanup for the existing host-exception contract. It requires a matching cleanup-enabled sysroot and dependencies, and JIR lowering for reachable cleanup bodies. Baseline fallback is an error when it would discard those bodies' cleanup paths. Rust panic/abort behavior remains unchanged. The pinned MIR bridge preserves the shared coroutine layout and applies rustc's abort policy to the body used by rustc.

Cleanup-enabled libraries carry a versioned, checksummed executable MIR member with stable definition, source and hygiene identities. Import validates the producer digest, rustc version, emit options, target layout and crate identity. Generic consumers can reconstruct that MIR without the original source files. A consumer given only a dependency's `.rmeta` (cargo's pipelining, which is how the sysroot's `alloc` sees `core`) reads the member from the rlib written beside it under the same stem; the identity checks prove it is the same compilation. A dependency whose rlib or member cannot be found is an error, never a fallback to rustc's optimized MIR, whose cleanup paths `-Cpanic=abort` removed. When two objects carry copies of one item, the link keeps a copy with cleanup edges over one without them, whatever their hashes, and keeps a copy that owes cleanup but declined encoding (its zombie fails a reachable use) over a body without cleanup edges. `scripts/cleanup-transport-test.py` checks cold/warm/edited builds, fresh consumers of warm libraries, incompatible producers, source absence, metadata-only dependencies with and without their rlib, the kept copy of a duplicated instance, and the cleanup edges of the sysroot `alloc`'s instances of `core` generics. The ordinary source-fact capture described below remains the default when executable cleanup is disabled.

Unset (or `JSC_PANIC=unwind`) selects the default unwind build mode; `JSC_PANIC=abort` is the opt-in `-Cpanic=abort` override. In unwind mode, the sysroot and every crate are compiled for `targets/mires32-unwind.json` (rustc's `wasm32-unknown-unknown` with `"panic-strategy": "unwind"`) with `-Cpanic=unwind`, and `js-cleanup=off` is appended after the caller's backend options, so a build that pins `js-cleanup=on` or a release profile runs in either mode. rustc's optimized MIR then keeps its unwind edges, and the backend lowers those: a JavaScript exception thrown by a host call runs the destructors of every live value on its way out and leaves the Rust frames as the same value, without the re-derived cleanup bodies of `js-cleanup=on`. The mode is keyed apart from every `abort`-mode sysroot (its panic strategy, the target spec's content and `patches/unwind/*.patch` join the cache key only in this mode); `patches/unwind/0001-core-eh-personality.patch` gives `core` the `eh_personality` lang item `-Cpanic=unwind` requires, so a crate needs nothing of its own (`examples/mini_core.rs` carries one for the `no_core` suite), and `patches/unwind/0002-stdlib-cold-nounwind.patch` marks `core`/`alloc`'s pure-Rust diverging panic/OOM helpers `#[rustc_nounwind]` so bounds checks and allocations do not emit redundant cleanup edges when Rust panics abort. `scripts/build_sysroot.sh`, `compile_core.sh`, `compile.sh`, `scripts/mires.mjs`, the experiment builders and `mires-build` read the switch (`JSC_RUST_MIR_INLINE=off` is the default client-crate MIR-inlining setting across those drivers unless `JSC_RUST_MIR_INLINE=on` or `--mir-inline on` is requested, while `build_sysroot.sh` builds the stdlib with `-Zinline-mir`); the receipts record it.

In the unwind mode:

* **A Rust panic is still an abort.** The `#[panic_handler]` runs, and every handler rethrows an abort without running cleanup (`__rt.is_abort`), exactly as under `-Cpanic=abort`; only a host exception unwinds. `catch_unwind` stays unsupported: the intrinsic is a zombie under any strategy but `abort`.
* **Every call a host exception could cross keeps its cleanup, or the program is refused.** rustc removes the cleanup edge of any call whose callee's ABI cannot unwind (rustc's own `fn_can_unwind`: `"C"`, `"system"`, `"cdecl"`, `"unadjusted"` and every other ABI except the `-unwind` spellings and Rust's own), so the destructors a function holds across such a call are gone from its MIR. The backend refuses, where the call is compiled, each such call that may let a host exception through while the function holds a value with drop glue across it (used after the call returns; for a call that does not return, any such value whose storage is live), and every message names the fix, `extern "C-unwind"`:
  * a call to a foreign import, with ``rustc_codegen_js: `name` is a foreign import with the "C" ABI, which cannot unwind: … Declare it `extern "C-unwind"`.`` and a note at the declaration (`examples/unwind-tests/290_extern_c_import_rejected.rs`, `296_cdecl_import_rejected.rs`);
  * a call through a function pointer of such an ABI, or of a `dyn` method declared with one (`295_extern_c_fn_pointer_and_dyn.rs`, `297_extern_c_import_through_pointer.rs`);
  * a call to a Rust function with such an ABI unless its transitive call and drop graph (tracked with cycle detection) is available and proven free of `UnwindTerminate(Abi)` edges, non-sysroot foreign imports, indirect calls (function pointers, `dyn` methods, or dropping a `dyn` value), inline assembly, tail calls, and unresolved or bodyless non-sysroot callees (`293_extern_c_fn_called_from_rust.rs`, `299_extern_c_fn_of_dependency.rs`, `310_transitive_extern_c_import_rejected.rs`, `311_transitive_c_rust_c_rejected.rs`, `312_transitive_c_fn_ptr_and_drop_rejected.rs`);
  * such a function inlined by rustc's MIR inliner, which leaves its `UnwindTerminate(Abi)` edges in the caller (`294_extern_c_fn_inlined.rs`).

  A call across which the function holds nothing to drop is accepted: the exception passes that frame as it would with the edge, and the frame above decides for itself. A Rust callee whose transitive call and drop graph is available and proven free of those hazards cannot let an exception out and is accepted, even across pure recursive cycles (`313_transitive_c_pure_cycle_accepted.rs`). The imports the sysroot's own crates declare are the runtime's and accepted; which crates those are is decided by where rustc loaded them from (the target's sysroot library directory), not by their names (`298_import_from_sysroot_named_crate.rs`). The sysroot build's own compilations (`-Zforce-unstable-if-unmarked`) are not checked: they call no host function, and `compiler_builtins`' intrinsics call one another through the C ABI; their generics are checked where a program instantiates them. It is a compile-time error rather than a zombie because the JIR lane replaces an item's zombies with its lowering's. `#[js_extern]` and `js_bindings!` expand to ordinary Rust functions, so their declarations are unaffected; `mires-js`, `view-abi` and the fixture preludes declare their imports `"C-unwind"`, which changes nothing under `-Cpanic=abort` (rustc gives the call `unwind unreachable` either way).
* **An exported `extern "C" fn` rethrows when JavaScript calls it.** Its cleanup path ends in `UnwindTerminate(Abi)`, which the JIR lowering emits as the same rethrow as `UnwindResume` once the drops have run (`UnwindTerminate(InCleanup)` still declines in a body with protected calls); a call to it from Rust is refused as above. The lowering is the same in both modes.
* **The baseline lane cannot build programs with cleanup.** Every body that keeps a cleanup block needs the JIR lane: a reachable one that the baseline lane would print is refused with the `requires semantic cleanup lowering` refusal of `js-cleanup=on` above, and the backend's fail-closed guard gives an item whose JIR encoding declined (or `js-jir=off`) the same zombie. A suite row that asks the baseline lane to build such a program is skipped in this mode with its reason printed.
* **Byte goldens are abort-mode text.** A golden recorded for the default mode is not compared in this mode; the behaviour checks beside it still run.

`scripts/unwind-mode-test.sh` runs the mode's own fixtures (`examples/unwind-tests/`) in whichever mode `JSC_PANIC` selects, with rustc's defaults and with `JSC_RUST_MIR_INLINE=on JSC_RUST_OPT_LEVEL=z`.

Backend options ride in on `-Cllvm-args`, space separated (`scripts/compile.sh` forwards
`$JS_EXTRA_ARGS`):

```text
js-names=readable|mangled    how identifiers are spelled; readable is the default
js-root=NAME                 an extra dead-code-elimination root, repeatable
js-structure=regions|trampoline   how control flow is rebuilt
js-queue=on|off              the expression queue
js-switch=dtree|flat         how `SwitchInt` is spelled
js-comments=on|off           the `// <def path>` header comment on each item (default on)
js-line-comments             `// file.rs:LINE` comments before each run of statements
js-source-map=on|off         write a `.map` beside the program (default off; see "Source maps")
js-emit-skip=PREFIX          hide items under a def path prefix (see "Emit goldens")
js-scoped-lets               declare locals at their region LCA
js-minify=on|locals|off      short names and compact printing (default off; see "Minify mode")
js-ecma-target=2018|latest   emitted JavaScript syntax ceiling (default latest)
js-hoist-consts=on|off       share repeated constants (default on; see "Hoisted constants")
js-modules=esm|script        ES module or plain script (default script; see "Modules")
js-shim-module=SPEC          where `__rt` is imported from (default `./shim.js`)
js-dom-module=SPEC           where the DOM runtime is imported from (default `topcoat-dom`)
js-profile=NAME              dev, report, release-size, release-balanced, or release-speed
```

`$JS_EXTRA_ARGS` is forwarded by **all three** compile paths: `scripts/compile.sh`,
`scripts/compile_core.sh` and `scripts/build_sysroot.sh` (which turns each space separated option
into its own `-Cllvm-args=`, since rustflags are split on whitespace).

`js-profile` is an alias layer: profiles expand before explicit options, so a spelled-out option always wins regardless of command-line order. The three release profiles select `js-ecma-target=2018`, the JIR lane, and optimizer emission; release-size and release-balanced use full minification, while release-speed keeps item names. Dev and report retain the `latest` syntax target.

**An option is emit-affecting when it can change the text of an item two crates might both
codegen.** That is the whole of the rule, and everything below follows from it: an option that
only decides what the *link step* synthesizes around the items, or that only reaches the leaf
crate's own output, is not emit-affecting however visible its effect on the finished file. The
module options are the worked example -- see "Modules".

**Every crate in one program must be compiled with the same emit-affecting flags.** The same
generic instantiation is codegenned in several crates, and the link step compares the two
spellings: if `core` was built with one set of flags and the crate linking it with another, the
identical function comes out as two different definitions and the link fails with
`two different definitions of ...`. Rebuild the sysroot after any change that alters emitted code
(readable local names did exactly this):

```sh
JS_EXTRA_ARGS='js-names=mangled' ./scripts/build_sysroot.sh   # then the same for the crate
```

Emit-affecting, so the sysroot has to be rebuilt to match: `js-names`, `js-structure`,
`js-queue`, `js-switch`, `js-scoped-lets`, `js-minify`, `js-ecma-target`, `js-hoist-consts` and
**`js-line-comments`** — the last one is
the surprise, because only the *leading* header comment run is stripped before two definitions are
compared (`LinkItem::code`), and its comments are interleaved through the body.

`scripts/build_sysroot.sh` records the options it built with in `build/sysroot/.js-args`, and
`scripts/test.sh` rebuilds the sysroot when they disagree with its own `$JS_EXTRA_ARGS`, so the
suite is green in every mode from one command. The diagnostic says so too: a
`two different definitions of ...` error now carries the note that names the cause.

Not emit-affecting, so they can be passed to one crate alone: `js-comments` (the header comment
run, which is exactly what `LinkItem::code` strips), `js-emit-skip` (acted on by the test script,
never by the backend), the three **module options** (everything they add is synthesized at link
time; see "Modules") and **`js-source-map`** (its markers print nothing; the JavaScript is
required to come out byte for byte identical). A map can therefore be asked for without rebuilding
the sysroot — a `core` built without one simply contributes items with no mappings, which then
read as generated glue and are stepped over.

`js-ecma-target=2018` is a syntax ceiling, not an ES2018-runtime claim. It prints BigInt constants through `BigInt("...")`, lowers logical and nullish assignment with exact one-evaluation semantics, and leaves runtime requirements such as `BigInt`, `globalThis`, `queueMicrotask`, `MessageChannel`, and `TextEncoder` in the separate final-artifact inventory. A pinned Acorn 8.15.0 parse of every public artifact is the compliance authority; Terser's `ecma` setting is policy metadata, and opaque `js!` input is accepted only when that final parse succeeds.

### Value representation

- `isize`/`usize` are exactly 32 bits and **are masked**, superseding the "no masking" line above:
  `>>>0` for `usize`, `|0` for `isize`.
- `&[T]` / `&mut [T]` is `{ buf, off, len }`: the backing JS array, a start offset into it, and a
  length. A subslice **shares `buf`** and only moves `off`/`len`, so a write through the subslice
  is visible in the original — which is the aliasing Rust guarantees, not an accident.
- `&dyn Trait` is `{ ptr, meta }`: the data value and the vtable.
- A vtable is a module level JS **array** in `tcx.vtable_entries` order:
  `[drop_glue_or_null, size, align, method, method, ...]`. rustc's own slot numbering is kept, so
  a virtual call is a single index — `InstanceKind::Virtual(_, idx)` already counts the three
  header slots. Vacant entries are `null`; a supertrait vtable is a nested array.
- `&*p` where `*p` is unsized is the **identity**: the fat pointer already carries everything a
  place of that type is (`buf`/`off`/`len`, or `ptr`/`meta`), so there is nothing to re-wrap.

### Aggregates assembled in place

MIR can give a local its value one field at a time, through projections, with no assignment of the
whole. rustc's clone shim for a tuple or a closure is that shape -- `_0.0 = <A as Clone>::clone(..);
_0.1 = ..; return` -- and so is every body its MIR inliner splices the shim into, where the return
place becomes the call's destination local. An aggregate here is a JavaScript array or object, so
such a local's storage is a container that has to be created, and created again whenever the
storage starts over: in its declaration at the top of the function, at its `StorageLive`, and after
it has been moved out whole, because the object that left is the moved value's now. MIR built
without storage markers has nothing else between two loop iterations that assemble one local and
move it into a `Vec`.

The container is `[]` for a tuple, an array or a closure environment and `{}` for a struct, a union
or an enum with a payload (`value::fresh_container`); a zero-sized value, a scalar and a transparent
wrapper around one need none, since a write to such a wrapper's field is a write of the whole. It is
made at one of those points only when a field write -- an assignment, a call destination or a
`SetDiscriminant` through a projection that does not start with a deref -- can follow before a whole
assignment, a `StorageLive`, a `StorageDead` or a whole move ends the question. A local assigned
whole before every field write it gets, which is all of the MIR rustc builds from source, has no
such point and is lowered exactly as before.

`backend/src/assembly.rs` answers this for the baseline lane over rustc's MIR and
`jsc-link/src/lower/assembly.rs` for the JIR lane over JIR-S. The JIR lane makes such a local a cell
and puts a `CellAlloc` at each point after the entry; a `CellAlloc` of a container-typed cell is
spelled as the empty container when the cell's storage is written in place (a mutable view rooted at
the cell with a path, or a `SetDiscriminant` on a load of it) before a store replaces it
(`fuse::fresh_cell_allocs`), and as nothing otherwise. Before this, the first field write threw `Cannot set properties of undefined`: every
clone of a `(String, usize)`, and every `Vec` of such tuples cloned.
`examples/core-tests/340_assembled_aggregates.rs` writes the shapes in custom MIR (the return
place for a pair, a triple, a struct, an array and an enum; a local assembled and moved in a loop,
with storage markers and without; assembled on one path and assigned whole on the other; assigned
whole once and assembled again after each move) and `341_clone_shim_aggregates.rs` pins the clone
shims themselves.

### Runtime shim additions

`runtime/shim.js` defines these beyond the print/abort members above:

```js
__rt.f2i(x, min, max)        // saturating float -> integer cast (NaN -> 0)
__rt.f2i_big(x, bits, signed)// the same for the BigInt widths
__rt.overwrite(target, value)// replace every own key of `target` with `value`'s
```

`overwrite` is what makes a whole-enum write correct: assigning a new variant into an existing
object has to *remove* the old variant's payload keys, not leave them next to the new ones.

### Calls to foreign items

A foreign declaration is only a shim call if its ABI says so. `extern "C"` (and friends) lowers to
`__rt.<symbol>(args...)`, as above. A declaration with the **Rust** ABI is not a foreign function
at all — it is a Rust symbol some other crate in the program defines, and the declaration exists
only because the definition cannot be named directly. Those lower to an ordinary direct call.
`core`'s `panic_impl` (the `#[panic_handler]`) is the case that matters; the allocator shims have
the same shape.

In the unwind build mode (`JSC_PANIC=unwind`, see "Compilation") a foreign import called while a
destructor is owed must be `extern "C-unwind"` (or have another ABI that can unwind): a call to an
import whose ABI cannot unwind loses its cleanup edge there, whether the call names the import or
goes through a function pointer, so the backend rejects it at compile time and names that fix. The
ABI's lowering is the same for both spellings.

### `#[panic_handler]` crates must not format

A `#![no_std]` crate's panic handler may read `PanicInfo::location()` — the `Location` arrives
through the `#[track_caller]` chain and its accessors are plain field reads. It must **not** touch
`PanicInfo::message()`: formatting a `fmt::Arguments` walks a byte-packed template through raw
pointer arithmetic, which has no meaning where a value is a JavaScript object rather than bytes at
an address. `patches/0001-panic-without-formatting.patch` applies the same rule inside `core`
itself, replacing the formatted runtime panic messages with their constant equivalents; without it
`panic_display` builds a `&dyn Display` whose vtable keeps every `Display` impl in the program
alive. The same patch stubs `str::slice_error_fail_rt`: that function used to format the
offending `char` with `Debug`, which kept `fmt`, escape iterators, and Unicode tables reachable
from every `&s[a..b]`. The abort is unchanged; the message is the constant
`failed to slice string`.

The rule is about the *handler*, not about formatting: `write!` into a `fmt::Write` sink runs, and
`examples/core-tests/11_fmt.rs` is the golden that pins it.

`patches/0004-char-searcher-byte-length.patch` removes a different statically impossible formatting root. `char::encode_utf8` returns a slice of the four-byte array supplied by `CharSearcher`, so that slice's length is in `1..=4` and stores exactly in the searcher's `u8` field. The patched source uses the direct cast instead of `TryInto::try_into().expect(...)`; this preserves every searcher state while keeping `TryFromIntError`, its `Debug` implementation, and the resulting `core::fmt` graph out of small `str::Lines` and `split_once` programs. `examples/core-tests/102_str_lines.rs` covers all four UTF-8 widths, and its `.absent` companion pins the deleted reachability.

`patches/0005-scalar-memchr.patch` selects `core`'s existing safe scalar implementation for forward `memchr`. The native aligned-word implementation has identical answers, but over this backend's JavaScript-array byte slices it adds pointer retyping, alignment, bounds, and panic machinery before falling back to scalar comparisons. The patch removes that target-inappropriate graph without changing reverse `memrchr`; `examples/core-tests/102_str_lines.rs` crosses the old length threshold and its `.absent` companion rejects `memchr_aligned`.

`patches/0006-char-searcher-nonpanicking-needle.patch` uses checked views for the encoded `CharSearcher` candidate and for `char` suffix removal after an explicit length guard. Those ranges are already established by the pattern state, so the safe checked form preserves the result without retaining `slice_index_fail`. `examples/core-tests/102_str_lines.rs` runs all four encoded widths, a mismatch, and a too-short suffix through a no-inline helper; its `.absent` companion pins the missing panic edge.

`patches/0007-lines-direct-split.patch` specializes only forward `Lines::next`: it drives the already-contained `SplitInclusive` state and removes the fixed ASCII `CR? LF` ending with safe byte checks and checked `str` views. The public `Map` representation, cloning, debug formatting, `size_hint`, and reverse iteration do not change. The same fixture covers LF, CRLF, empty, and unterminated lines and rejects the unused forward `Map::next` adapter.

`patches/0012-char-searcher-scalar-prefix.patch` specializes the fixed-width comparison inside forward `CharSearcher::next_match`. `memchr` has already matched the encoded character's final byte, so exact one- through four-byte checked slice patterns compare only the preceding bytes and avoid generic slice equality. The search state, reverse path, and mismatch behavior are unchanged. `examples/core-tests/102_str_lines.rs` covers every width, same-final-byte false candidates, an interior false hit in U+A041, and negative cases.

### Not supported (each is a zombie, reported only if reachable)

Mutating a `&mut str` · unsizing a struct with more than one field · `simd_*` intrinsics ·
`f16`/`f128` · `global_asm!`.

Reinterpreting a buffer at another element size — `align_to`, and the chunked reads built on it —
is **not** a zombie but a run time refusal (`__rt.scale`/`__rt.unscale`/`__rt.chunk_slice`). It has
to be: the unreachable-at-run-time fast path of `str::count::do_count_chars` is on the static call
graph of every `Display for str`, so a zombie there would reject `write!` itself.

### Names

`backend/src/naming.rs` invents every item name and `backend/src/names.rs` every local name;
nothing else in the backend builds an identifier.

- An item is a sanitized def path with `::` spelled `$`, plus an unconditional `$h` and 16 hex
  digits of a hash of its v0 symbol. The hash is not collision *repair* — it is what makes a name
  depend on nothing but the item itself, so two crates spell a shared item identically.
  `#[no_mangle]`/`#[export_name]` items keep their exact name.
- A local is named after the body's `var_debug_info` where it says something (`sum`, `acc`, `i`)
  and `_0`, `_1`, ... where it does not. `_0` is always the return place. Parameters are named the
  same way, in the signature and at every use.
- The `$` prefix is the backend's own namespace — `$t0` (hoisted temporaries), `$s0` (structurizer
  selectors and bound scrutinees), `$loc` (the `#[track_caller]` location), `$v` (accessor setter
  parameter). A Rust identifier cannot contain a `$`, so nothing a user writes can collide.
- A splatted `"rust-call"` tuple parameter takes `<name>$e0`, `<name>$e1`, ...; two locals that
  want the same readable name are distinguished with `$1`, `$2`, numbered per function.
- Invariants the link step depends on: an item name never starts with `$`, never matches `^_[0-9]`
  and is never `bb`; a local name obeys the same three rules and additionally never shadows `__rt`
  or a JS builtin the emitted code reads.
- `-Cllvm-args=js-names=mangled` restores raw v0 symbols and numbered locals, which is how a
  naming bug is told apart from a lowering bug. **It cannot carry a program whose sysroot has both
  `core` and `alloc`**: rustc appends the *instantiating crate* to the symbol of a cross-crate
  generic instance, so `core` and `alloc` spell one shared instantiation two ways, and the items
  that call it then differ. Readable names do not have the problem — their hash is taken over the
  instance's crate-independent stable hash, which is what [`Namer::hash_key`] exists for — so
  `js-names=mangled` is now a mini_core and single-dependency tool.
- `-Cllvm-args=js-minify=on` replaces both halves with generated names — `$a` for an item, `a` for
  a local — in two disjoint namespaces. See "Minify mode".

### Emitted JS shape

- Each item may be preceded by a `// <def path>` header comment (`js-comments`, on by default).
  The comment belongs to the declaration that follows it: the blank line goes **before** the
  comment, never between it and its declaration.
- `js-line-comments` adds `// <file>:<line>` before each run of statements lowered from the same
  Rust line — the cheap, readable-in-a-golden-file cousin of a source map. The path is spelled as
  rustc would spell it in a diagnostic, so `--remap-path-prefix` applies.

### Statics that refer to each other

A `static` is one module level `let NAME = <literal>;`. Its value was const-evaluated, so the
literal only *names* the bindings it points at (a reference to a static is the binding, or
`{ buf: NAME, off: 0 }`, or `__rt.box(NAME)`), and never reads what they hold. The link step
declares a binding after the bindings it names, a depth-first post-order.

Rust allows statics that name each other: two nodes, a ring, a static that names itself, through
fields, enum payloads, arrays and slices of references. No order of plain `let`s initializes those
(the first declaration reads a binding still in its temporal dead zone, a `ReferenceError` before
any Rust code runs), so **a cycle is initialized in two steps**: every member is declared as an
empty object (an empty array for an array literal), then each member's literal is evaluated and
copied onto it with `Object.assign`. The binding keeps the identity it was declared with, which is
the one every other initializer and function captured, so `ptr::eq` of the two ways to reach a
member is true, and a write through one is seen through the other. The cycle sits where the
post-order puts it as a whole: after what its members name outside it, before every binding that
names a member. Module evaluation runs initializers only, so no Rust code sees a half-filled
cycle. A program without a cycle prints byte for byte as before.

A cycle whose member is not one `let` of one object or array literal, or that runs through a
binding of another kind, is a link error naming the statics, never a reordering that throws at
load. So is a member whose literal has a `__proto__` key: that is what a Rust field named
`__proto__` prints as, an object literal takes it as the prototype rather than a property, and
`Object.assign` copies properties only, so the filled object would lose the field. (Outside a
cycle the same key is a printer gap of its own: a scalar `__proto__` field reads wrong.) See
`jsc-link/src/link/statics.rs`; `260_static_cycles`, `261_static_cycle_cells`,
`267_static_cycle_arrays` (array members are real arrays: copied by value they are `.slice()`d)
and `268_static_cycle_proto_key` pin it.

### Source maps (`js-source-map=on`)

A **source map v3** is written beside the finished program, so a browser steps through the Rust
rather than through the JavaScript.

- Artifacts: `<out>.js.map` next to `<out>.js`, and `//# sourceMappingURL=<basename>.js.map` as
  the **last line** of the JavaScript. Both output paths produce it: the single crate
  `--emit=obj` path (`link.rs::prune_objects`, what `scripts/compile.sh` uses) and the real link
  path (`link.rs::link`, what `scripts/compile_core.sh` uses). Object files inside an rlib never
  get one — nothing steps through an intermediate — so the map comment can never collide with the
  `//# rcgjs:` item table footer, which only object files carry.
- **The JavaScript does not change.** With the option on, the output is byte for byte what it was
  with the option off, plus the trailing `sourceMappingURL` line. The locations ride in the AST as
  `Stmt::Loc` markers, which print nothing; the peepholes that pattern match on statements
  (`else if` chains, `while` recovery, destination passing, the trailing `return`) all look
  through them on purpose.
- Granularity is one mapping per **run of statements from one Rust position** — the same seam and
  the same de-duplication as `js-line-comments`.
- `sources` are file names as rustc spells them in a diagnostic (`--remap-path-prefix` applies).
  `sourcesContent` combines the current session with source text retained in dependency objects.
  Object section S7 stores compressed text independently of JIR bodies, including generic-only
  crates, so dependency source trees need not be present at link time. Archive packaging refreshes
  a separate source member on every build, independently of codegen-unit reuse. The linker decodes it only
  for source-map output and rejects conflicting text under one remapped name. Sources unavailable
  from legacy objects remain `null`.
- `names` maps an emitted name to the Rust one: a function's def path, and every local whose
  spelling had to change (`self$1`). A fresh mapping is opened immediately after each such
  identifier, because a debugger reads the name off whatever mapping covers it.
- Every generated line is mapped. A line with nothing of its own repeats the mapping in effect —
  Firefox stops a mapping at the end of its line — and a line at the start of an item, or before
  any location at all, maps to the synthetic source `<rustc_codegen_js generated>`, which is the
  only entry in `ignoreList`. That is the blackboxing rule: the prologue, the dispatch scaffolding
  and the missing-item stubs are code no Rust programmer wrote, and a debugger told so steps over
  them.
- `scripts/check-sourcemap.js <out.js> [source-substring]` checks a map without a browser: JSON
  shape, every VLQ segment, every index in range, every generated position inside the file, every
  original position inside its source, every line mapped, and `ignoreList` naming only glue.
  `demo/index.html`'s header comment has the manual DevTools recipe.
- Byte budgets (`NN.maxbytes`) are **not enforced** by `scripts/test.sh` when `$JS_EXTRA_ARGS`
  asks for `js-source-map`, `js-line-comments` or `js-minify`: the budgets are baselined against
  the default mode, and moving them to fit another mode would stop them catching what they exist
  for. Sizes are still reported.

### Minify mode (`js-minify=on`)

`-Cllvm-args=js-minify=on` shortens names and drops the whitespace. Off by default; **with it off
the emitted JavaScript is byte for byte what it was before minification existed**, which
`scripts/test.sh` and `scripts/emit-test.sh` between them pin on 48 runtime fixtures and 20 JS
goldens.

Three things happen, in two places:

- **Local names** (`rcgjs-object/src/minify.rs`, at codegen time, per item): a function's parameters,
  `let`s, `$`-temporaries and labels become `a`, `b`, `c`, ... Parameters take the alphabet
  positionally first, then the remaining names are taken in order of falling occurrence count,
  each given the shortest name still free — js_of_ocaml's strategy, measured there within 0.06% of
  a mixed-integer-programming optimum, with no interference graph. Ties break on the name, which
  makes the output a function of the input alone. The alphabet is jsoo's: 54 characters may start
  an identifier and 64 may continue one, encoded little-endian.
- **Item names** (`rcgjs-object/src/minify.rs`, at link time, once): every reachable item that is not
  exported becomes `$a`, `$b`, `$c`, ... by the same frequency rule. This *has* to be
  whole-program — an item is codegenned by whichever crate instantiated it and called from any
  other — and by then an item is text, so the pass is a scan rather than a parse. It is sound
  because it substitutes only exact matches of names in the link step's own item table, and skips
  string literals, `//` comments and identifiers after a `.` (a property, a different namespace).
  The text is not arbitrary JavaScript but this backend's printer output, which has no regular
  expression literals, no template literals, no single quoted strings and no block comments.
- **Compact printing** (`jsast::Style::Compact`): no indentation, no newlines inside a
  declaration, no spaces that only separate tokens, no `;` before a `}`, no braces around a
  single-statement `if`/`while` body that cannot itself be an `if` (which is what keeps the
  dangling-else problem out), and `!0`, `!1`, `void 0` for `true`, `false`, `undefined`. Where two
  tokens would fuse — `a - -b`, `a + +b`, an accidental `//`, the `<!` and `-->` of an HTML
  comment — a space goes back in; that table is jsoo's `need_space` and is checked on the
  characters that actually met, not on the AST.

The two namespaces are disjoint **by construction**: a generated item name always starts with `$`
and a generated local name never does. That is what lets the two passes run in different places
without either having to know what the other did. Beyond that, a local never takes a name the
function it is in already mentions (so it cannot shadow an item, `__rt` or a JS builtin), never a
reserved word, and never one of the globals in `minify.rs::GLOBALS`.

What keeps its name: **exported items** (`#[no_mangle]`, `#[export_name]`) at both passes —
`counter_clicked` is called from `demo/index.html` and `rust_entry` from `scripts/entry.js`, and
renaming those would be renaming the program's interface. An export short enough for the allocator
to reach (three characters or fewer) is warned about at link time, because it is the one name the
local renamer cannot see; this is the same hazard `names.rs` documents for readable local names.

- `js-minify` is **emit-affecting**, so the sysroot's `core` has to be built with it too — see
  "Compilation". `scripts/test.sh` now keeps the two in step by itself.
- `js-minify` turns `js-comments` **off** (a header comment is the larger half of a small item).
  It composes with `js-source-map`: local renaming moves the map's emitted-name keys with the AST,
  and whole-program item renaming rebases generated columns before the final map is encoded.
- `js-minify=locals` is the bisecting step: compact printing and short local names, item names left
  alone. A minified program is then still greppable for a def path, and it is how the two halves of
  the win were measured apart.
- Item `refs` sets are left exactly as codegen computed them. They are over-approximations
  intersected with the item table, and local renaming cannot change which *item* names an item
  mentions; recomputing would only swap one set of ignored entries for another.

`scripts/measure-size.sh` produces the table below — raw and gzipped bytes, because a program is
served compressed and a long name repeated a hundred times costs a hundred back-references rather
than a hundred names. Five mini_core fixtures, three core fixtures and the demo:

```text
FIXTURE           readable     gzip    locals     gzip    minify     gzip gz cut
01_arith              8249     1313      6835     1115      2924      889    32%
04_enums              4512      958      3422      815      1714      630    34%
08_closures           7324     1478      5951     1233      1994      683    54%
12_bigint             9750     1547      8414     1341      3547      915    41%
14_slices             8760     1751      6734     1409      3331     1139    35%
01_option            19952     3314     14638     2577      8620     1675    49%
04_iter_range        72435    11424     48767     8400     24406     4850    58%
09_closure           24351     4962     17728     3782      9029     2348    53%
counter                806      375       509      294       343      238    37%
```

The mini_core rows are smaller than they were before `--remap-path-prefix` reached
`scripts/compile.sh` (`01_arith` was 9094/3769) and the gzip columns are slightly larger than they
were before constant hoisting; "Hoisted constants" has the per-flag breakdown.

The `locals` column is why the item pass exists. Compact printing plus short locals is a 15-25%
gzip cut on its own; shortening the item names on top of it is worth another 15-35 points, because
a readable item name is 20 to 60 characters and 30-60% of the raw bytes of a program. Post-gzip it
still dominates: the compressor turns the repeats into back-references but pays for each name once,
and there are hundreds of them.

### Emit goldens (examples/emit/)

`scripts/emit-test.sh` compiles each fixture in `examples/emit/` with pinned flags and checks two
expectations: `NN.expected` (stdout under node) and `NN.js.expected` (the emitted JavaScript,
byte for byte). Both, always — a golden JS file on its own would let a beautifully formatted
miscompilation through. `UPDATE_EXPECT=1 scripts/emit-test.sh` rebaselines.

`js-emit-skip=PREFIX` is a **display** rule, applied by the script to the emitted text and not by
the backend: the file that runs is the unfiltered one, because an item the backend actually left
out would be a `ReferenceError` rather than a tidier golden. It exists so that the `mini_core::`
prelude every fixture drags in does not rebaseline all fourteen fixtures whenever mini_core
changes.

A fixture may carry two more companion files. `NN.skip` replaces the def path prefixes hidden from
the golden, and `NN.core` marks a fixture compiled against the real `core` and `alloc` in
`build/sysroot` rather than against mini_core. `17_alloc_shim` is the pair's reason for existing:
the allocator shim is the one thing in a program no crate has a body for, so pinning it needs a
program that actually allocates, and its skip list hides everything the standard library brought
along. `scripts/module-test.sh` passes over a `NN.core` fixture — the invariant it holds is a
property of codegen rather than of what a crate depends on.

`14_pointers` is the stage 1.5 addition: it pins the *shape* of the pointer model — a slot
dereferenced on both sides of an assignment, an offset folded into the index it produces
(`p.buf[p.off + n]`), a primitive local boxed because its address is taken, and the loop every
monomorphized `slice::Iter` becomes.

## Demo (demo/)

counter.rs: same no_core header + mini_core; a `Counter` struct with
`fn increment(&mut self)` and a `#[no_mangle] fn counter_clicked(n: i32) -> i32`
pure handler (state lives in JS). index.html: loads shim.js and counter.js (script tags),
a button and a span; clicking calls `counter_clicked(current)` and renders the result.
No server needed — file:// friendly.

## Pointers (stage 1.5)

A pointer is a **slot**: `{ buf, off }`, a container plus a key, where `buf` is a JS array or
object and `off` is a number index or a string property key. `off` counts **units of the pointee
type**, never bytes. Reading through a pointer is `p.buf[p.off]`, which is an assignable JS place,
so a write through a pointer is an ordinary assignment and every alias sees it. Every operation
that produces a pointer either names a real slot or degrades to a provenance free number, and
nothing in between.

`rcgjs-object/src/ptr.rs` (the record shape) and `backend/src/ptr.rs` (every reader and writer
that consults a layout) are the one place that builds and consumes these records; every other
module goes through them. It is also where the model is being landed: an operation that does not implement
the specification below says so in its doc comment, and the shim helper behind it throws instead of
answering.

### Forms

| form | shape | what it is |
|---|---|---|
| slot | `{ buf, off }` | a pointer to one place, dereferenceable |
| scaled slot | `{ buf, off, sc }` | a byte offset view of a buffer of `sc`-byte elements, from an `as *const u8` cast; **not** dereferenceable |
| prefix slot | `{ buf, off, pr, ps }` | a pointer to a non-zero-sized field at layout offset zero; `pr` retains the containing slot and `ps` its pointee size |
| window slot | `{ buf, off, w }` | a pointer to `w` consecutive elements, from an `as *const [E; N]` cast |
| fat slice | `{ buf, off, len }` | `&[T]`, `*const [T]`, unchanged from stage 1 |
| fat dyn | `{ ptr, meta }` | `&dyn Trait`, unchanged from stage 1 |
| fat tail | `{ ptr, meta }` | a pointer to a **struct with an unsized tail**; see below |
| address | a JS number | `null()`, `NonNull::dangling()`, `without_provenance`, an integer transmuted to a pointer, and a pointer in a constant with no provenance behind it; never dereferenceable |
| object | the pointee's own JS value | a reference to an aggregate, which keeps its identity |
| string | a JS string | `&str` |

A slot record is **immutable**: an offset produces a new record rather than moving `off`, which is
what keeps `Operand::Copy` of a pointer an alias rather than a clone.

### References and raw pointers

The two differ for aggregate pointees only.

- `&T` and `&mut T` of an aggregate are the aggregate's own JS object. This is what keeps the
  emitted code readable, and it is not negotiable.
- `*const T` and `*mut T` of an aggregate are a slot: the slot naming the place the reference was
  read out of (`{ buf: points, off: i }` for `&raw const points[i]`), which is what keeps a pointer
  to one element of an array able to reach the next. A raw pointer taken from a bare object goes
  through `__rt.box`, which keeps one slot per object in a `WeakMap`, so two raw pointers taken
  from one object are the same record and `ptr::eq` on them is true. The reverse, a reference from
  a raw pointer, is `p.buf[p.off]`.
- A pointee that is a JS primitive (a number, a `bool`, a `char`, a function pointer) has no object
  to point at, so **every** pointer to it is a slot: a local whose address is taken is boxed
  (`let x = [init]`, used as `x[0]`, with a parameter re-bound in the function prelude), and so is
  a static of such a type.

Two rules about boxing that are easy to get wrong, and both are miscompilations when they are:

- **A borrow the backend never lowers is not a borrow.** A `view-abi` marker's `&root` argument is
  read at codegen time and the assignment that computed it is dropped ("Templates" below), so that
  borrow is not in the emitted program and its local needs no box. `uses.rs` counts twice for this:
  once to find the dropped assignments, then again discounting the borrows they take. Only the
  boxing flag is discounted; every count stays as MIR wrote it, so a local such a statement mentions
  is still neither inlined nor treated as immutable.
- **A boxed return place is read through its box.** The return place is a local like any other and
  can be boxed, so `return` reads `_0[0]`, and destination passing (folding the last assignment to
  `_0` into the `return`) is off for one: the box may still be named by a slot the body handed out,
  so the store into it has to stay.

### Dereference

| pointer form | `*p` reads | `*p = v` writes |
|---|---|---|
| slot | `p.buf[p.off]` | assignment to `p.buf[p.off]` |
| object | the object itself | `__rt.overwrite(p, v)`, so every alias sees it |
| fat slice, fat dyn | the record; the projections that follow read `buf`/`off`/`len` or `ptr` | as for the pointee it names |
| fat tail | the record at `p.ptr`; `p.meta` is kept for the projection that reads the tail | as for the record it names |
| zero sized pointee | `undefined` | nothing is stored, but the value is still evaluated |
| `&str` | the string | a zombie: `&mut str` has no mutable JS form |
| scaled slot, address | a run time error | a run time error |

`&*p` where `*p` is unsized stays the identity: the fat pointer already carries everything a place
of that type is.

An element of an array or slice whose element type is zero-sized, and anything projected out of a
zero-sized array, is a place with **no storage**: a zero-sized array is `undefined` like every
zero-sized value, so there is no buffer to index. Reading such a place is `undefined` and writing
it stores nothing. The element count is unaffected: it is the array type's `N` or the slice's
`len`, so bounds checks, `len`, and compiler drop glue's loop over `&raw mut (*s)[i]` run exactly as
many times as natively.

Every pointer to such an element is **one address, the one its container provides**:

* an element of a zero-sized array `a` is at `a`'s own pointer: `&a`, which is `undefined` for a
  local or a field (the reference form of a zero-sized value), or the raw pointer `a` was reached
  through;
* an element of a slice of zero-sized elements is at the slice's data pointer, which the record
  keeps unnormalized as its `buf`: `{ buf: undefined, off, len }` when it was unsized from such an
  array, `{ buf: d, off: 0, len }` when `from_raw_parts(d, len)` built it (a `Vec<Z>` passes its
  dangling address `d`). `off` is not part of the address.

The conversions answer the same address: `as_ptr`, `as_mut_ptr` and every cast of a `*const [Z]` to
a zero-sized pointee is `buf`; `add`, `offset` and `wrapping_add` on a zero-sized pointee do not
move it; `from_raw_parts(p, n)` is `{ buf: p, off: 0, len: n }`, so it round trips. `ptr::eq`,
`ptr::addr_eq` and `as usize` therefore agree for `&a[i]`, `a.as_ptr()`, `s.as_ptr()`,
`s.first()`, an iterator's item, and a `Vec<Z>`'s `as_ptr()` against its elements. `undefined` as a
pointer has the address `2^29` (`__rt._zst_addr`, never given to a buffer), so `is_null()` is false
for every one of them and every alignment predicate holds. Rust leaves it unspecified whether two
different zero-sized arrays share an address; here they do when both are `undefined`. Not
supported: `==`, `ptr::eq` or ordering of two fat `*const [Z]` / `&[Z]` (the comparison reaches
`__rt.ptr_eq`, which refuses a `buf` of `undefined`; `ptr::addr_eq` on the same slices works), and
`offset_from` on a zero-sized pointee, which panics in Rust.
`examples/core-tests/114_zst_aggregate_drops.rs` and `115_zst_element_pointers.rs` are the
fixtures.

### A struct with an unsized tail

A struct whose **last field is unsized** is unsized itself, and it is neither a slice nor a trait
object: it is a record with a run of elements at the end of it. A pointer to one is fat, and it
carries the record rather than the tail:

```js
{ ptr: <the record>, meta: <the tail's length> }
```

The same two keys a `&dyn Trait` uses, for the same reason: what the pointer names is one value
and the metadata beside it is what says how much of it there is. A reference carries the record;
a raw pointer carries its slot. Unsizing preserves that distinction.

`core`'s `array::IntoIter` is why this exists, and it is what `for x in [a, b, c]` compiles to. It
holds a `PolymorphicIter<[MaybeUninit<T>; N]>` -- `struct PolymorphicIter<DATA: ?Sized> { alive:
IndexRange, data: DATA }` -- and unsizes a reference to it to `&mut PolymorphicIter<[MaybeUninit<T>]>`
for every operation. The unsizing coercion reads the **lockstep tails** for the metadata only
(`[T; N]` to `[T]` gives `N`) and leaves the record alone; fattening the tails themselves would
hand back a fat slice over the whole record, which the dereference on the other side then reads as
the struct it is.

Three projections read the shape, and they have to agree:

* a `Deref` names the record at `p.ptr` and keeps `p.meta` for later;
* the projection that reads the **tail field** is what turns the two halves back into a fat
  pointer: the field holds the elements and the metadata counts them, so the tail is
  `{ buf: p.ptr.data, off: 0, len: p.meta }`. Only a **slice** tail is built this way, because
  `[T; N] -> [T]` is the only coercion that makes a record with a tail out of a sized one. The
  steps that can sit between the two -- a transparent field, a downcast, a cast that changes
  nothing -- produce no JavaScript and carry the metadata along;
* `PtrMetadata` is `p.meta`, and `size_of_val` is the sized prefix plus `meta` elements of the
  tail.

`examples/core-tests/28_array_into_iter.rs` is the fixture.

#### The heap half

A `Rc<[T]>` allocates one of these rather than coercing to one, and the block it allocates is a
header plus the tail. A JavaScript array holds one element type, so the block is laid out with the
header **as element 0** and the tail after it:

```js
[ { strong, weak }, t0, t1, ..., t(len-1) ]
```

and the side table remembers where the tail starts (`hdr`). `*mut RcInner<[T]>` is then
`{ ptr: { buf, off: 0 }, meta: len }` -- the same two keys again -- and the two projections fall
out of the layout: `strong` is `p.ptr.buf[p.ptr.off].strong`, and the tail is the ordinary fat
slice `{ buf: p.ptr.buf, off: p.ptr.off + 1, len: p.meta }`. Every `{ buf, off }` the model already
builds works over that tail unchanged, which is the point of putting the header first: an offset is
a new record, a write is an assignment, a subslice moves `off` and `len`.

So the `.ptr` half is the record for a **reference** and a slot for a **raw pointer**.
`tail_ref` and `tail_raw` preserve metadata and the exact containing slot across that boundary.
`tail_slice` uses that slot to recover an allocated tail from the original block; a coerced sized
record keeps its original tail field. Both paths share storage without copying elements.

The reshape is `__rt.retype_rc(p, header_mk, tail_es, len)`, and it accepts the two states `alloc`
actually arrives in: `allocate_for_slice` casts the fresh block to the element type first, so the
block has already been uniformly retyped, while `allocate_for_ptr_in` goes through
`with_metadata_of` and arrives byte granular. `header_mk` is a factory for the header record, for
the same reason the zero element factory is one: only the compiler knows what the sized fields of
`RcInner` are and what a zero one of each looks like.

`examples/core-tests/29_rc.rs` is the fixture: the sized surface first, as the pin, and the tail
after it.

Only a **slice** tail is built. A `str` or a `dyn` tail is a run of nothing a JavaScript array can
hold, and it is refused with the type named
(`examples/core-tests/guard_rc_str.rs`); `Rc::into_raw`/`Rc::from_raw` are refused with it, because
`byte_sub(data_offset)` steps backwards across the header boundary and this model has no offset
that crosses from a tail element to the record in front of it.

### Casts

One function owns the matrix, `ptr::cast_pointer`, and every pointer to pointer cast reaches it:
`PtrToPtr`, `FnPtrToPtr`, `ArrayToPointer`, `MutToConstPointer` and the pointer arms of a
`transmute`.

| from -> to | result |
|---|---|
| either side is a `str`, from a slice of one byte elements | `__rt.bytes_str`; see "`str` is a hybrid" |
| same size, same indirectness | identity, which covers `*const T` to `*mut T` and the **layout-identical newtype** below |
| zero sized pointee on the target side | the **reference form** of the source pointee: `&T`, which is the object for an aggregate and the slot for a primitive |
| zero sized pointee on the source side | the way back, the raw pointer that reference form denotes |
| aggregate to its single non zero sized field | reproject: `{ buf: p.buf[p.off], off: "<field>" }` |
| larger aggregate to a matching non zero sized field at layout offset zero | `__rt.prefix`, which retains the containing slot and therefore the shared address |
| that zero-offset field back to its containing aggregate | `__rt.unproject`, which accepts only the prefix slot made by the forward cast |
| size N to size 1 | `__rt.scale`: a scaled slot over the same buffer, and an address unchanged |
| size 1 to an integer, float or `char` of size N | `__rt.unscale_scalar`: allocation retyping for a fresh heap block, scale recovery for a scaled slot, or an aliasing little-endian scalar view over byte storage |
| size 1 through a transparent union to size N | `__rt.unscale_uninit`: `unscale` plus one-time retyping of array-backed uninitialized stack storage |
| any other size 1 to size N | `__rt.unscale`, which retypes a fresh heap block or throws |
| element to array of elements | a window slot |
| array of elements to anything | `__rt.unwindow`, then the element's answer |
| anything else | a zombie at the cast, naming both types |

The array rows come before the size rows, and `__rt.unwindow` is a question asked at run time: a
pointer to `[E; N]` is either a plain slot whose one element is the whole array or the window slot
an earlier cast produced over `N` consecutive elements of somebody else's buffer, and nothing in
the type says which.

A zero-offset prefix cast is not the single-field transparent case. An aggregate may have other
fields after the prefix, as `alloc::collections::btree::node::InternalNode<K, V>` does after its
`LeafNode<K, V>`. The projected JavaScript place is the prefix field, but both Rust pointers have
the same address. The prefix slot therefore retains the original slot. Addressing, equality,
ordering and a zero byte round trip use that retained provenance. Pointer arithmetic may not leave
the prefix field, and the reverse cast checks both the retained slot and field key, so a standalone
value with the prefix type cannot be promoted into a larger containing aggregate. A same-size,
one-field newtype is not a prefix and continues through the ordinary transparent or allocation
cast rows.
`examples/core-tests/101_btree_map_nodes.rs` pins both directions through the standard library's
B-tree node types.

An integer, `f32`, `f64` or `char` read through a suitably aligned byte pointer gets a cached view
whose indexed properties decode and encode the source bytes in little-endian order. The view is an
ordinary slot to the compiler, so a write through it is immediately visible through the byte
pointer and a repeated cast preserves pointer identity. Its synthetic address and a cast back to
bytes continue to use the original buffer. Width alone is not enough to reconstruct a pointer,
enum or aggregate, so those types keep the refusing `unscale` path.
Building a raw slice through `slice_from_parts` normalizes a dangling data address. The data half
is asked by its value, never by its type: `with_metadata_of`, which is how `Box::clone_from_ref`
(`Box<[T]>::from(&[T])`) rebuilds its fat pointer, hands `from_raw_parts` the data pointer as
`ptr as *const ()`, and a pointer to `()` says nothing about what the value is. Only a *reference to
an array* is its own buffer; the JIR lane used to take that shortcut for every pointer whose form
was the reference form, built a slice over the pointer record, and read every element of such a box
wrong before refusing to free it (`examples/core-tests/342_boxed_slice_from_ref.rs`). Creating a
`u8` or `i8` slice reference through `byte_slice_ref` seals initialized heap storage as bytes. A later scalar view aliases that storage
without changing its byte capacity, including a partial word at the end. Cached views acquire new
word slots after heap growth. Raw slices and `MaybeUninit<u8>` slices leave allocation retyping
available. Fixture 110 covers empty
vectors, slice offsets, UTF-8 word reads, invalid sequences, and growth against native Rust.
The custom sysroot deliberately keeps forward `memchr` scalar rather than routing JavaScript-array bytes through these word-sized views. `examples/core-tests/102_str_lines.rs` pins that absence; direct scalar alias reads and writes remain part of the pointer contract above.

`[u8; N]` and `uN` still convert little endian **as values** in the transmute lowering rather than
through a pointer view. That is the form `u32::from_le_bytes` and `fmt`'s
`cast_array().read()` actually reach. A *pointer* cast to the scalar uses the aliasing accessor
above; a pointer cast to an array at another element width remains unsupported.

A union with a single non zero sized field (`MaybeUninit<T>`) is **transparent**: it is represented
as that field, and `uninit()` is `undefined`. So is a **`repr(transparent)` struct**, which the
language guarantees is exactly its one non-1-ZST field: `NonNull<T>` *is* the pointer inside it,
`ManuallyDrop<T>` is its `T`, `NonZero<u32>` is a number. An ordinary newtype keeps its object
form: the guarantee is what this reads, not the field count.

One more shape is read the same way, and it is an **observation rather than a guarantee**. A
**layout-identical newtype** — a `#[repr(packed)]` struct with one non zero sized field, at offset
zero, whose size equals that field's — holds exactly its field's bytes and differs from a
`repr(transparent)` one in a single respect: its alignment is 1 rather than the field's. This model
has no alignment, so there is nothing left for the wrapper to be that its field is not, and it is
represented as its field.

`core::ptr::Unaligned<T>` is why the row exists. `read_unaligned` casts a `*const T` to a
`*const Unaligned<T>`, reads it and transmutes the wrapper away; `write_unaligned` wraps and writes.
Both become the identity — but **only** because the two sides also agree on the JavaScript *value*.
Answering the pointer cast alone would leave `write_unaligned` storing a `{ _0: n }` object into a
place the rest of the program reads as a number, which is a silent wrong answer rather than a
rejection. So the rule lives in the value model, where construction, field reads, cloning and the
cast all read it, and the cast row above then follows for free.

The check is deliberately narrow. `pack` is what separates this from every ordinary newtype: a
plain `struct Meters(f64)` is layout-identical to its field too and keeps its object form, because
nothing about it says the two are interchangeable.

`read_unaligned` still does not reinterpret a buffer. Reading four bytes of a `[u8; N]` as a `u32`
asks for an element that buffer does not have, and `__rt.unscale` refuses it at the cast whether the
read is aligned or not; `u32::from_le_bytes` is the spelling that works.

The two halves differ in one thing, and it is what the directness rule reads. A **union**'s value
may be nothing at all, so a transparent union is **direct** whatever its field is — a reference to
one is a slot rather than an object, for the same reason a pointer record is. That is what makes
`mu.write(v)` an assignment to `p.buf[p.off]`, which works over an uninitialized place and is seen
by every alias of it; a reference that were "the object itself" would have nowhere to point. A
transparent *struct* always holds its field, so it is direct exactly when the field is, and
`ManuallyDrop<Point>` is the object a `Point` is.

The chain is why the struct half matters. `core`'s `MaybeUninit<T>` is a union over
`ManuallyDrop<T>`, which is a `repr(transparent)` struct over `MaybeDangling<T>`, which is another
— so `MaybeUninit<u8>` is the bare number `5`, and a `[MaybeUninit<u8>; N]` is an array of bytes
that `slice_assume_init_ref` and `str::from_utf8_unchecked` can read as the bytes they are owed.
With the wrappers left opaque it was `{ value: { _0: 5 } }`, and every buffer `core::fmt` builds
its digits in held objects where bytes belonged.

### Addresses

`p as usize` is `__rt.addr(p, size_of::<pointee>())`. Every buffer is given a synthetic base the
first time it is asked about, kept in a `WeakMap` so that it never moves, and the address of an
element is `base + off * size`; a scaled slot's offset is already in bytes and is added as it is,
and a number is an address already.

A `usize` is 32 bits, so the space of bases is finite and is managed like a buddy allocator. A
buffer of `n` bytes takes a granule of the smallest power of two that holds it, at least 16 and at
most 4096, and its base is a multiple of that granule — aligned for every type the buffer can
hold, since a type's alignment divides its size. A heap block's size is its `Layout` size; another
buffer's is its length times the element size of the first `addr` asked of it. An alignment the
size does not imply — a hand-built `Layout` whose alignment exceeds its size, or an over-aligned
element first addressed through a byte pointer — is honoured only up to the buffer's granule
(at least 16). Granules come from a
per-size free list first, then from a bump pointer starting at 4096 (whose alignment gaps are
split into smaller free granules). `dealloc` gives a heap
block's granule back — the freed buffer keeps its entry, so a dangling pointer still answers the
address it had, and a later allocation may reuse it, as a native allocator would — and a
`FinalizationRegistry` gives back the granule of any other buffer once the engine has collected
it. A `realloc` that grows a block past its granule (below 4096) gives the granule back too, and
the block takes a base at its new size when next asked: a reallocation may move in Rust, and
otherwise the grown block's addresses would run into its neighbours'. A base is therefore unique
among live buffers and stable for a buffer's lifetime. The space runs out only when the bump
pointer reaches 2^32 with no free granule of the size asked — about 2^32 bytes of granules live at
once, a million live 4 KiB buffers — and that is a refusal that says so. (Bases used to be `4096 * n`
and were never reused: the 2^20th buffer was given the address 2^32, which `>>> 0` made null, and
`NonNull::new` reported a good allocation as failed.)

The size does not always survive to the call. `<*const T>::addr` is written
`transmute(self.cast::<()>())`, so the pointee is `()` by the time the address is taken while the
offset is still counted in `T`s — which is why the erasing cast (`__rt.erase`) records the element
size on the record as `es`, and `addr` reads it in preference to the size it was passed. `es` is
inert everywhere else: a dereference is still `p.buf[p.off]`, which is what lets the erased pointer
`core::fmt` hands to a formatter be the `&T` that formatter expects.

The scheme is chosen for four guarantees, not for convenience:

- `is_null()` is false for every real pointer and true only for `null()`, because no base is zero;
- `NonNull::dangling()` is an alignment, always below 4096, so it never collides with a base;
- an address is a multiple of the element alignment (a type's alignment divides its size, and a
  buffer's base is aligned to its size rounded up to a power of two), so alignment predicates answer
  correctly;
- `fmt::Arguments::as_str` tests `bits & 1` on an 8 byte element, so the answer is deterministic.

Addresses are **not** ordered across buffers in any way a program may rely on, and an address never
turns back into a dereferenceable pointer.

### Element offsets

`off` counts pointee units, which makes the offset operations exact in both worlds:

- `ptr::add`/`offset` produce a new record with `off + count`, written out inline so that the
  dereference that follows folds back to `p.buf[p.off + n]`;
- `ptr::wrapping_add`/`arith_offset` go through `__rt.offset(p, count, size)`, which tests the form
  at run time. That is the split: Rust requires the *strict* offset to stay inside an allocation, so
  a pointer that is an address cannot reach it without undefined behaviour, while the wrapping form
  is exactly what a program is allowed to use on `null()` or a dangling pointer — a slice of a zero
  sized element counts its length that way. An address moves by `count * size` and stays a number;
- `ptr_offset_from` is `a.off - b.off`, and `byte_offset_from` is `(a.off - b.off) * size`. An
  **aggregate** pointee goes through `__rt.offset_from` instead, because one of the two records may
  not know where it is: a reference to an aggregate is the object, so a raw pointer made from one
  that arrived as a value is a `__rt.box`, a buffer of one. The helper finds that object in the
  other pointer's buffer by identity — `choose_pivot` returns one of three `&T`s and asks which
  index it was, so every sort of a struct depends on it;
- `copy`, `copy_nonoverlapping` and `write_bytes` move and fill runs of *elements*: `count` is an
  element count for a slot, and a byte count over a scaled slot, which is divided by `sc`. An
  aggregate element is cloned on the way across, because a byte for byte copy is an independent
  value and assigning a JavaScript object would be an alias;
- an element move **writes through** (into the object already in the place, `__rt._put`) only
  where the element is an aggregate held by value: the move carries a `clone`, or the destination
  is a `__rt.box` buffer (the one place standing for a local), or it is a byte copy over a scaled
  slot, whose `u8` pointee says nothing about the elements. Every other element **rebinds** the
  place. A reference to an aggregate is that aggregate's object and a raw pointer or a `Box` is a
  slot record, so writing one into the object in the place would overwrite its *pointee*:
  `[&h, &h2].swap(0, 1)` would copy `h2` over `h`;
- `write_bytes` names a value for two pointees only: a one byte one, where the pattern *is* the
  value, and a zero pattern on any other scalar. An aggregate is a zombie, and a non-zero pattern on
  a wider pointee throws;
- `compare_bytes` goes through `__rt.compare_bytes`: two unscaled byte buffers compare exactly, so
  the byte length is the element count and the bytewise `PartialEq` of `str` and `[u8]` works; two
  scaled slots of equal `sc` compare element by element over `count / sc`, which is what makes
  `[i32] == [i32]` work; a mismatched pair throws;
- `raw_eq` is `===` on two scalars, and `every` over an array of them — a fixed size array compares
  through a single `raw_eq` of the whole array, so without that arm no `[T; N] == [T; N]` would
  work. An array of scalars has no padding, so element by element asks what the bytes would.

`__rt.ptr_eq` compares data pointers through object identity or their `buf`, `off` and `sc`.
Equality of wide pointers also compares metadata: `slice_ptr_eq` checks `len`, and `fat_ptr_eq`
checks the data half and `meta`. Casting to a thin pointer discards metadata. `ptr_cmp` compares
`off` when both sides share a buffer and the addresses otherwise, and returns -1, 0 or 1.

Zero-length copies, byte writes and byte comparisons access no storage. Their operands are still
evaluated, and Rust's alignment and non-null preconditions still apply. An uninitialized allocation
can be cast to its complete-element prefix without losing its full byte size; trailing bytes do
not become initialized values.

### `str` is a hybrid

`&str` stays a JS string, and the byte view is produced on demand:

- `as_bytes` is `__rt.str_bytes(s)`, which encodes the string to UTF-8 and returns
  `{ buf, off: 0, len }` over a **plain array**, the same kind of buffer every other `[u8]` is —
  exactly the bytes `TextEncoder` writes, so a lone surrogate is U+FFFD. `as_ptr` is the same call
  reduced to the slot `{ buf: __rt.str_bytes(s).buf, off: 0 }`, because a thin pointer carries no
  length.
- The records are **memoized for the operation**, keyed by the string. `&s[a..b]` is
  `s.as_bytes()` plus a decode, so a parser walking a document asks for the whole document's bytes
  once per token; the memo makes that one encode per operation instead of one per token (linear
  rather than quadratic). Its bound: the memo is emptied when the JavaScript stack unwinds (the
  first insertion queues one microtask that clears it, so no entry outlives the synchronous run of
  Rust code that made it — an export, a callback, one poll of a future); it holds at most 4 MiB of
  encoded bytes, and the insertion that would exceed that empties it first; a string of more than
  1 MiB is held apart from that budget, at most the two most recent such strings, so a large
  document is not evicted by its own slices. Without `queueMicrotask` only the byte bound applies.
  Strings are immutable, so an entry is never stale — a mutated `String` derefs to a different
  JavaScript string — and an owned `String` built from a host string copies, never aliasing a
  memoized buffer. A repeated call within the operation returns the same record, so pointer
  identity holds in practice; identity across independent calls is **not** guaranteed, and nothing
  may depend on it.
- `from_utf8_unchecked`, and a raw pointer whose pointee is `str` (`str::from_raw_parts`), are
  `__rt.bytes_str(buf, off, len)`. An `i8` element is read as its byte.
- The metadata of a `&str` is `__rt.str_len(s)`, the UTF-8 byte length, computed from the UTF-16
  code units without encoding (a surrogate pair is four bytes, a lone surrogate the three of
  U+FFFD); a string of more than 64 code units is answered from the memo when its bytes are
  already there. `str::len` lowers to the same call: `__rt.str_bytes(s).len` folds to
  `__rt.str_len(s)`.
- A `String` the optimizer keeps native (a host string, or the builder `{ text, cap, len }`, see
  OPTIMIZER.md) holds exactly the text the baseline `Vec<u8>` `String` would decode, because
  every piece that joins it is well formed on its own. A piece that may come from a host — a
  `push_str`/`write_str` argument, each `&str` argument of a `write!`/`format!` site, a host
  string converted into a builder — first goes through `__rt.native_str_wf`, which turns each
  lone surrogate (only a host can hand Rust one) into U+FFFD, the three bytes `str_bytes` stores
  for it. Every other piece is well formed by construction: a pushed `char`, a format template's
  literal text, the ASCII text of an integer, `bool` or `f64`, a `to_string` result, a baseline
  string converted into a native carrier, and a native `String` argument's text, itself only
  ever built from such pieces. (A `char` that a host function returns is not checked to be a
  scalar value; a surrogate forged that way is outside this guarantee, in the baseline
  representation as well.) Two adjacent pieces therefore never pair into a scalar the baseline
  never held — two appended halves of a surrogate pair stay two U+FFFD, in one site or across
  appends — and the host is handed the same string either way.
- The builder never asks `str_len` about the text it has accumulated: `len` is that text's exact
  UTF-8 byte length, kept by every operation (an append adds only the appended piece's length),
  so `String::len` is a field read and building a string is linear in the bytes appended. `cap`
  is the Rust-visible capacity in the same bytes, never below `len`: it starts at the requested
  capacity (zero, or the exact length for a builder made from text) and grows exact-fit, to the
  new length, when an append exceeds it — not by `RawVec`'s amortized doubling.
- Writing through a `&mut str` stays a zombie: the `&mut str` a `String` derefs to is a decoded
  JavaScript string, so a write through `as_bytes_mut` (`make_ascii_uppercase`,
  `to_ascii_lowercase`) does not reach the `String`'s storage and is lost. It is contained: a cast
  from `str` to a **mutable** byte pointer is `__rt.str_bytes_mut(s)`, a fresh record that is never
  memoized, so the lost write never surfaces in the view another `as_bytes` of an equal string
  reads.

`ptr::cast_str` owns the whole matrix — `str` to `[u8]`, `str` to `u8`, a slice of one byte elements
to `str`, `str` to `str`, and a zombie for anything else with a `str` on one side — and every
pointer cast asks it before the size based matrix, because the sizes say nothing about a JavaScript
string.

The slice being decoded into a `str` may have **any one byte element**, not only `u8`. A one byte
slice is the one buffer whose element count and byte count are the same number, which is what lets
its `len` be read as a byte length, and two types qualify -- both of which are buffers of plain
JavaScript numbers, so `__rt.bytes_str` reads either:

- `u8` and `i8`. A signed element is accepted because the byte is the same byte either way; the
  decoder reads it through a `Uint8Array`, which is where the sign goes.
- a **fieldless enum with a one byte integer `repr`**, which *is* its discriminant (see
  "Enums"). A fieldless enum *without* an integer `repr` is a variant name rather than a byte, and
  is not a one byte element at all.

`core`'s `AsciiChar` is the second shape, and `[AsciiChar]::as_str` is the cast. It is on the static
call graph of every `char` formatted with `Debug`, and so of `str::slice_error_fail` — which means
of `&s[a..b]`, `contains`, `find` with a `&str` pattern, and the float formatter. All four work
because of this row.

The same enums **transmute to and from the integer of their own width**, as the identity; see
"Transmuting between an enum and an integer" under "Enums". That is how `escape_ascii` builds its
output (`u8` to `AsciiChar`) and how `core::mem::Alignment` is read as a `usize`.

### Runtime shim additions

```js
__rt.box(value)              // the slot a raw pointer to a bare aggregate names
__rt.prefix(p, key, size)    // project a zero-offset aggregate field and retain its provenance
__rt.unproject(p, key)       // recover the exact containing slot from that prefix pointer
__rt.addr(p, size)           // the synthetic address of a pointer
__rt.ptr_eq(a, b)            // whether two pointers name the same place
__rt.ptr_cmp(a, b, size)     // -1, 0 or 1
__rt.offset(p, count, size)  // the wrapping offset, for a pointer that may be an address
__rt.copy(dst, src, count, clone?)             // overlapping element move
__rt.copy_nonoverlapping(dst, src, n, clone?)  // the same without the overlap
__rt.write_bytes(dst, byte, count, zero?)      // repeat a byte pattern
__rt.compare_bytes(a, b, count)        // lexicographic byte comparison
__rt.read_array(p, n)        // the `n` elements at `p`, as a JS array
__rt.write_array(p, values)  // the write half of read_array
__rt.unscale(p, size, mk)    // a scaled slot back to a slot, throws if misaligned
__rt.unscale_scalar(p, size, mk, kind) // an aliasing integer/float/char view over byte storage
__rt.unscale_uninit(p, size, mk)       // one-time retype of uninitialized array storage
__rt.chunk_slice(p, n, w)    // a slice re-read at chunk granularity, throws if it is not one
__rt.unwindow(p)             // a pointer to `[E; N]`, as a pointer to its first element
__rt.thin(p, size)           // `from_raw_parts` with `()` metadata: the data pointer, un-erased
__rt.str_bytes(s)            // UTF-8 bytes of a JS string as `{ buf, off, len }`, memoized
__rt.str_bytes_mut(s)        // the same bytes in a fresh, unmemoized record (`as_bytes_mut`)
__rt.str_len(s)              // UTF-8 byte length
__rt.bytes_str(buf, off, len) // UTF-8 bytes back to a JS string, `""` at length zero
```

`__rt.thin` is what `from_raw_parts` answers with when the pointee is **sized**: its metadata is
`()`, so nothing is being rebuilt and the result is the data pointer itself. The work is in the
erasure that pointer arrived through. `from_raw_parts` takes a `*const impl Thin`, and
`with_metadata_of` — which is how `byte_offset`, `byte_add` and `byte_sub` are all written — hands
it `self as *const ()`. For a `*const u8` that value is already the slot; for a `*const i32` it is
the scaled byte view `cast::<u8>()` produced, and keying it back into elements is the whole of the
round trip. Nothing in the type says which, so the record is asked, the same way `__rt.unwindow`
asks. `ptr_metadata` of a thin pointer is `undefined`.

### Still not supported

Each is a zombie, reported only if reachable: `ptr_mask` ·
byte punning into an aggregate or a semantic scalar other than the supported integer, float and
`char` byte views · `raw_eq` on an
aggregate other than an array of scalars, and `write_bytes` on any aggregate · writing through a
`&mut str` · reprojecting a pointer from a scalar to a wrapper around it, unless the wrapper is a
[layout-identical newtype](#casts) · reinterpreting an array at another element width (`[u16; 8]`
to `[u8; 16]`) · a `transmute` between an integer and a fieldless enum with **no** integer `repr`,
whose value is a variant name and so carries no discriminant to observe (see "Enums") ·
a struct with a **`str` or `dyn` tail** (`Rc<str>`, `Arc<dyn Trait>`), where a slice tail is
[supported](#the-heap-half) because only a run of elements can be laid out after the header ·
`Rc::into_raw` and `Rc::from_raw`, whose `byte_sub(data_offset)` steps backwards across that
header · `TypeId` equality · SIMD · `f16`/`f128` · threads.

What is **no longer** on that list, and was: `&s[a..b]` on a `str`, `contains`, `find` with a `&str`
pattern, `Display` and `LowerExp` for `f32`/`f64`, `{:?}` on a `char`, `escape_ascii` and
`escape_debug`, `str::parse::<f64>()`, `read_unaligned`/`write_unaligned`, and
`byte_offset`/`byte_add`/`byte_sub`. `Debug` as a whole is still out — `{:?}` on a `char` works
because it is reached as a concrete impl, while `#[derive(Debug)]` and `Result::unwrap` coerce to a
`&dyn Debug` and drag every `Debug` impl in the program into reachability.

#### The zombie census in `core` and `alloc`

The count is measured off the rlibs the sysroot was built from, which is where a zombie is
recorded: each object carries a `//# rcgjs:` footer, and the recorded messages are read back out of
it. **Seven** survive in `core`, in six groups, and every one is on this page. The census was
re-measured after the enum representation changed, after the unsized-tail reference form landed and
after its heap half did. `core` has not moved at any of the three: the "no integer `repr`" transmute
row above is reached by nothing in `core` or `alloc`, and neither half of the tail representation is
reached from `core` at all.

| count | zombie | why it stays |
|---|---|---|
| 2 | `[u16; 8]` -> `[u8; 16]` (`Ipv6Addr`) | reinterpreting an array at another element width |
| 1 | `[u8; 16]` -> `[u16; 8]` (the way back) | the same |
| 1 | `[u8; 16]` -> `u128` (`TypeId`) | `TypeId` has no stable byte representation here |
| 1 | `*const ()` -> `[u8; 4]` | byte punning a pointer |
| 1 | `NonZeroCharInner` -> `char` | reached only from that type's `Debug` |
| 1 | `*const str` -> `*const ()` | a `str` is a JavaScript string, whose only byte view is the one `as_bytes` produces |

`alloc` adds **twenty**, in five groups, and none of them is reachable from `Box`, `Vec`, `String`,
`format!` or `Rc` -- which is what the core suite demonstrates:

| count | zombie | why it stays |
|---|---|---|
| 6 | `core::ffi::CStr` (size, metadata, rebuilding one from a thin pointer) | an unsized pointee whose metadata this model has no form for |
| 5 | `core::wtf8::Wtf8` | the same |
| 4 | pointer constants of type `*const ()` | a pointer in a constant with no provenance behind it |
| 3 | `RcInner<Wtf8>` / `ArcInner<Wtf8>` | a struct whose tail is neither sized nor a slice, so it has no run of elements to lay out |
| 2 | `*const str` -> `*const ()` | as in `core` |

The fourth row is what is left of the Rc group, and it is the boundary rather than the whole
feature: a heap block **is** reshaped into a header plus a slice tail now
([above](#the-heap-half)), so `Rc<[T]>` and `Arc<[T]>` work and the five casts that used to sit
here are down to the three whose tail is a `Wtf8`. `Rc<str>` is the same row from a program's side
and `examples/core-tests/guard_rc_str.rs` pins it.

None of them is reachable from string handling, formatting or slices, which is what the earlier
counts were dominated by. In particular there are **no** `u8 as char` zombies: that arm has been
answered in `rvalue.rs` and `intrinsics.rs` for some time, and any note claiming otherwise is stale.

Some failures are deliberately left to run time rather than reported as zombies, because the
program is only wrong if it reaches them: rebuilding a slice at another element granularity —
`align_to`, `as_chunks`, `array_chunks`, `chunks_exact` — which would need elements the buffer does
not have, and which has to be a run time refusal rather than a zombie because `align_to` is on the
static call graph of `Display for str` through `do_count_chars`, so rejecting it would reject
`write!` itself (`examples/core-tests/guard_as_chunks.rs` pins the abort, and the same guard is
what reverse `str::rfind(char)` reaches through `memrchr`); dereferencing a scaled slot or an address; `__rt.unscale`
on an offset that is not a multiple of the element size; a `copy` or a `compare_bytes` between two
pointers whose scales disagree, or a byte count that is not a whole number of elements; a run that
reaches past the end of its buffer, which is undefined behaviour in Rust and here is the shape a
byte count over a buffer that does not hold bytes would take; `write_bytes` of a non-zero pattern
over a pointee wider than a byte; and an `addr`, `ptr_eq` or `ptr_cmp` on a record the *strict*
offset built from an address, which is the one silent wrongness the model has and is why those
three ask.

### A reference to a fixed size array

`&*p` where the pointee is `[E; N]` has to be an array **object**, because that is what a reference
to an aggregate is here. Which JavaScript value that is depends on the record, not on the type, so
`__rt.array_ref(p, n)` asks it, exactly as `__rt.read_array` and `__rt.unwindow` ask their version of
the same question:

* a **plain slot** holds the whole array as its one element, so that element is the answer. A place a
  write is about to put an array in -- a fresh heap block, which is what `Box::new([1, 2, 3])`
  allocates -- has nothing there yet, and handing back what is there is still right, because the
  write reaches the place through the pointer rather than through this.
* a **window** slot (what `as *mut [E; N]` produces) and a **slice** record (what
  `<&mut [T] as TryInto<&mut [T; N]>>` leaves its `len` on) both name `n` consecutive elements of
  somebody else's buffer, and at any offset. There is no array object there to hand back, so one is
  made whose `n` elements are `Object.defineProperty` accessors onto those places: a real array by
  `Array.isArray`, `length` and indexing, and a write through it is seen by every alias of the
  buffer, which a copy would not be. Memoized by buffer and offset, so two references to the same
  elements are the same object and pointer identity holds.

`itoa` is why this matters and `examples/core-tests/25_array_ref.rs` is the fixture: `itoa::Buffer` is
a `[MaybeUninit<u8>; 40]`, `Buffer::format` casts a pointer to it down to a shorter array, and the
integer writers convert `&mut buf[1..]` back to a fixed size array at a non-zero offset. Both reach
the aliasing case. Before this the emission was the plain-slot answer for all three shapes, which
silently handed the callee ONE ELEMENT where the array belonged: `serde_json` could parse integers
and not print them.

## Enums (stage 3)

An enum's JavaScript shape follows from its **declaration**, never from how a value of it is used,
so every part of the backend can ask the question of a type and get the same answer.
`backend/src/value.rs`'s `EnumRepr` is that question and this section is its specification.

| the enum | its JavaScript value | example |
|---|---|---|
| every variant fieldless, **with** an integer `repr` | the discriminant, as a plain **number** | `Ordering::Less` is `-1`; `AsciiChar::A` is `65` |
| every variant fieldless, **no** integer `repr` | the variant's name, as a bare **string** | `Sign::Pos` is `"Pos"` |
| any variant carries a payload | an **object**: `TAG` plus the active variant's fields | `Some(3)` is `{ TAG: "Some", _0: 3 }`; `None` is `{ TAG: "None" }` |

Tuple-variant fields are keyed `_0`, `_1`, ... as everywhere else. A field literally named `TAG`
is emitted as `TAG_` (`naming.rs`), so a struct variant can never overwrite the tag.

Three properties are worth stating, because code elsewhere depends on each:

- **A mixed enum keeps the object form for its fieldless variants too.** `Option<T>` is
  `{ TAG: "None" }`, not `"None"`. An enum with a payload is [indirect](#value-representation): a
  reference to one **is** its JavaScript object, and `*p = v` overwrites that object in place
  (`__rt.overwrite`). A bare string has no identity to overwrite, so `Option::take`, `replace` and
  `insert` -- every `&mut Option<T>` in `core`, and so every iterator adapter -- would silently fail
  to write through the reference. The alternative, making mixed enums *direct*, is correct but boxes
  every borrowed `Option` local in the program.
- **The encoding is total.** The empty variant is a value, never an absent one. There is no
  `None = undefined` sentinel, so a niched `Option<&T>` and an `Option<Option<&T>>` are told apart
  by construction, and an uninitialized place (which *is* `undefined`) is never mistaken for one.
- **A fieldless enum is a JavaScript primitive**, so it is **direct**: a reference to a local of one
  is a `{ buf, off }` slot and the local is boxed (`uses.rs`), exactly as for a `bool`. It also
  needs no copy (`Operand::Copy` of one is the value) and compares with `===`, which is what lets
  `==` on a fieldless enum be lowered at all. A zero sized enum stays indirect like every other ZST.

### Reading a discriminant, and switching on one

MIR types the result of `discriminant(x)` as an integer, so the backend has to decide per read
whether the **tag** or the **number** is wanted. `backend/src/tag.rs` is one pre-pass per body that
answers it:

- a local whose single definition is `discriminant(x)` of a tagged enum with more than one possible
  variant, and whose **every** use is a `SwitchInt` scrutinee, reads the tag: the value itself for a
  fieldless enum, `x.TAG` otherwise;
- **every other** discriminant read is the number, spelled by a chain over the variants
  (`x.TAG === "None" ? 0 : 1`). `E::A as i32` and `mem::discriminant` -- which may be hashed as well
  as compared -- both go through it, and for a `#[repr(int)]` enum it costs nothing because the value
  already *is* the number.

The classification is conservative towards the number, which is correct everywhere and merely
longer. It requires more than one *possible variant* rather than more than one variant: a layout
with one inhabited variant has no tag in the value at all, and `ControlFlow<Infallible, T>` -- two
variants, one layout, reached by every `?` in the program -- is exactly that case.

A `SwitchInt` over a tag becomes a `switch` on strings, with each case value looked back up as the
variant whose discriminant it is:

```js
switch (s.TAG) {         // switch (s) for a fieldless enum
  case "Nothing": ...
  case "Num": ...
}
```

The structurizer's decision tree is unchanged and still reports *ordering* tests (`Lt`, `Le`,
`InRange`) whenever the values it is partitioning happen to be contiguous -- an or-pattern like
`A | B => ..` really does produce one. Names have no order, so `emit.rs` spells those by listing the
variants the test accepts, as an `||` chain of `===` (or, negated, an `&&` chain of `!==`). The
numeric decision tree is untouched for integers, for `char`, and for `#[repr(int)]` enums.

### Transmuting between an enum and an integer

A fieldless enum with an integer `repr` **is** its discriminant, in this model as on a real machine,
so a `transmute` between it and the integer of its own width and signedness is the **identity** --
in both directions, and to a thin pointer as well (`NonNull::dangling()` is a transmute of a
`mem::Alignment` straight to a pointer). The widths and the signedness have to agree exactly.

A fieldless enum **without** an integer `repr` is a variant name, and the discriminant such a
transmute would be observing is not part of the value. No rule claims it, so it is a
[zombie](#the-zombie-census-in-core-and-alloc) rather than a guess. Nothing in `core` or `alloc`
reaches one: every fieldless enum they transmute has an integer `repr`.

An enum with a payload is an object, and a **two-variant niche-encoded** one is the other shape
`core` transmutes. From an integer it is decoded at run time -- the niche value is the empty
variant, anything else is the payload (`NonZero::new`, `Option<&T>`, `Option<char>`) -- and to an
integer it is encoded the same way round: the empty variant is its niche value, re-signed at the
integer's width, and the other is its one payload field transmuted into the integer. The conditions
are the same in both directions (the tag as wide as the integer, one niche variant carrying no data,
one payload field), so the two round trip. `PartialOrd::lt` and its siblings for `[T]` need the
encode: `core` writes them as a transmute of a `ControlFlow<bool>` to `u8` (`slice/cmp.rs`,
`as_underlying`), which made `a < b` on slices, and every sort of slices, a zombie before.
`examples/core-tests/344_slice_partial_ord.rs` pins both directions.

The same rule is what makes a slice of one byte elements readable as text: a `[u8]`, an `[i8]` and a
slice of a fieldless one-byte `#[repr(int)]` enum are all buffers of JavaScript numbers, and
`__rt.bytes_str` reads all three. `[AsciiChar]::as_str` is the cast that needs it, and it sits on
the static call graph of every `char` formatted with `Debug`.

### The size of it, and why it ships by default

Measured with `scripts/measure-size.sh` and the dom suite, against the numeric-tag encoding it
replaced:

| | raw | gzip |
|---|---|---|
| measure-size table, readable | +1.5% | **+1.3%** |
| measure-size table, `js-minify=locals` | +1.9% | **+1.5%** |
| measure-size table, `js-minify=on` | +3.9% | **+2.3%** |
| the counter island (all modes) | 0.0% | **0.0%** |
| the 14 dom fixtures + the shim + `topcoat-dom` | -0.5% | **-0.5%** |

A variant name costs more than a small integer, and it lands entirely on enum-dense code: the
worst case is `04_enums`, a fixture that is nothing but enums, at +7.5% minified-gzip on a 630 byte
file. It is paid back by `mem::Alignment` ceasing to be an object, which deletes three separate
wrappers from the allocator path every `Vec`, `String` and `format!` program carries -- the shim's
`typeof` unwrap, the `Object.assign` clone chains around `layout.align`, and the two-sided `.$t`
comparison. That is why the dom suite came out smaller. Nothing that contains no enums moved by a
byte.

So the encoding is the default and there is no flag to turn it off: the regression is inside the
budget the change was gated on, the emitted JavaScript says what it means, and a string-tag union is
a shape a `.d.ts` can name.

## Hoisted constants (stage 1.5)

A value the program writes over and over is emitted **once**, as a module level `const`, and named
at every use. Two kinds qualify:

- **`#[track_caller]` locations.** `intrinsics.rs::caller_location_value` interns each distinct
  `(file, line, column)` under the key `loc\x01<file>\x01<line>\x01<column>` and emits
  `const loc$h<hash> = { file: "...", line: N, column: M };`, with the identifier at the call.
- **String literals of 32 bytes or more** (`constant.rs::HOIST_STRING_BYTES`), keyed by the text
  itself: `const s$h<hash> = "...";`. The threshold is not a break-even point for a string used
  once — it is where the strings that repeat start paying (panic messages, `unsafe` precondition
  texts). A JavaScript string is a value, so sharing one can never be observed.

The mechanism is the vtable machinery unchanged: `CguCx::hoist` calls `CguCx::intern`, so the item
is built once per codegen unit, and the use site is the constant's *identifier*, so `JsItem::new`
picks the edge up mechanically and the reachability pass keeps the constant alive exactly as long
as something names it. The new `ItemKind::Const` is a binding (`is_binding()`), so the link step
emits it before the functions that read it. A hoisted constant carries **no header comment** even
with `js-comments=on`: its debug path is its value spelled a second time.

**Determinism, and why cross-crate sharing works.** The name comes from `Namer::synthetic_name`,
whose FNV hash is fixed across compilers and crates, so two crates that hoist the same value emit
an item with the same name *and* the same text — which is the case `link.rs` drops silently. That
holds only if the two crates spell the key the same way, so the key may never contain a
`def_path_str` (`naming.rs` says why), and the file in a location key must be remapped
identically. Both compile scripts now pass `--remap-path-prefix "$root/="`, and the toolchain's
own `core` sources are already spelled `core/src/...`; the core suite exercises the result — a
location in `core` reached from a generic instantiated in both crates is emitted by both and
appears once in the program.

`-Cllvm-args=js-hoist-consts=off` restores the inline form, byte for byte, for bisecting. It is
**emit-affecting**: the sysroot has to be built with the same setting as the crate linking it.

Hoisting is a size win in raw minified bytes only where a value repeats. Measured against
`js-hoist-consts=off`, it costs 1-4% of the *gzipped* size everywhere, because a 16 hex digit hash
in a name is incompressible while the repeated literal it replaced was a back-reference; and it
costs raw bytes too on a fixture whose locations are all used once (`14_slices`: +5.6% minified).
The larger win in this area was `--remap-path-prefix`, which took ~90 bytes off every location
literal in the mini_core suite. A per-item reuse count (hoist a value an *instantiation* mentions
twice, which stays deterministic across crates, rather than one the codegen unit mentions twice,
which does not) is the follow-up that would make the flag pay in both metrics.


## Allocation (stage 3)

`alloc` is in the sysroot, and `Box`, `Vec` and `String` work. Everything below is how.

### The block

A heap block is a **JavaScript array**. `__rt` keeps a side table of what else is known about it,
a `WeakMap` keyed by the array itself, so a block nothing points at is collected with its entry:

| field | what it is |
|---|---|
| `bytes` | the byte capacity the `Layout` asked for |
| `es` | the size in bytes of the elements the array currently holds |
| `fresh` | whether the block has been [retyped](#retyping) yet |
| `zeroed` | whether it came from `alloc_zeroed` |

A pointer into a block is an ordinary `{ buf, off }` slot, so every operation the pointer model
already has works over it unchanged: an offset is a new record, a write is an assignment, a slice
is `{ buf, off, len }` over the same array.

A block is **born byte granular** — `es` is 1 — because a `Layout` is a size in bytes and nothing
at the point of allocation says what will be put there.

### Retyping

A block is **retyped exactly once**, at the first `*mut u8` to `*mut T` cast made while it is still
fresh. The array's length becomes the element count the byte capacity buys (`bytes / size`) and
`es` becomes that size.

This is sound because the bytes are *uninitialized*: there is nothing in them to lose. It is also
the only moment at which it is sound, which is why the fresh flag exists. `RawVec::allocate_in` and
`Box::new` both allocate and immediately cast, so the retype always happens before anything is
written.

The rule is one step wider than that, and the second step is what a `Rc<[T]>` needs. A block may be
**reshaped** into a header plus a tail -- element 0 the header record, the tail after it, `hdr` on
the side table -- and the full invariant is:

> at most one uniform retype, plus at most one struct reshape while the block is provably unwritten.

"Provably" is meant literally: `__rt.retype_rc` checks that every element really is `undefined`
before it moves anything, so a block something has already written to is a refusal rather than a
silently rebuilt one. The two steps are separate because `alloc` performs them separately --
`allocate_for_slice` casts the fresh block to the element type and only then asks for the struct --
and a reshape that arrives at a block still holding bytes is accepted too, because `with_metadata_of`
is the other way in. See "A struct with an unsized tail" under "Pointers" for the layout.

Asking for the size the block **already has** is free and is the common case, not the exception:
`RawVecInner` stores its buffer as a `*mut u8` and casts it back on every single access, so a `Vec`
runs through the cast once per element read.

### Uninitialized stack storage

The standard library also uses aligned stack arrays of `MaybeUninit<u8>` as scratch storage and
casts them to `MaybeUninit<T>`. Stable sorting is the representative path. The transparent union at
the source says that no byte value is observed through the old element shape, so an array-backed
stack buffer may acquire an element size exactly once without becoming a heap allocation.

`__rt.unscale_uninit` keeps the same `bytes`, `es` and `fresh` facts in a separate weak side table
and applies the ordinary retype rules. Repeating the same element size is free; changing it after
the first retype, using a misaligned offset, or presenting non-array storage is a run time refusal.
The separate table is important: stores into sort scratch space must not acquire allocator
ownership or deallocation behavior. `examples/core-tests/100_uninit_stack_retype.rs` pins this
path.

### Collecting in place

`alloc` reuses the allocation of a `vec::IntoIter` for the `Vec` an adapter chain over it collects
into, whenever the destination element fits in the source's (`vec/in_place_collect.rs`). That
reinterprets a block of source elements as a block of destination elements while the rest of the
source is still being read, through a cast between the two element pointers. A block here holds one
element representation and is retyped at most once, so that cast has an answer only when the two
types share a representation; otherwise the reuse was a compile-time refusal (`(u32, usize)` to
`usize`), a run-time one (a newtype to its field) or a buffer projected into its first element (a
one-tuple to its field). `patches/0014-no-in-place-collect.patch` makes `in_place_collectible`
false, so `into_iter().map(..).collect()` and its relatives always collect into an allocation of
their own, as they do natively whenever the layouts do not fit. The result's `capacity()` is the
one that path gives (the iterator's lower size bound) rather than the source's.
`examples/core-tests/343_into_iter_collect.rs` covers the shapes.

### The zero element factory

A block from `alloc_zeroed` holds zero *bytes*, and a zero element is not always the number `0`: it
is `false` for a `bool`, `{ TAG: "None" }` for a niched `None`, `"A"` for a fieldless enum whose
zero discriminant is `A`, `{ x: 0, y: 0 }` for a struct. Only the
compiler knows which, so **the cast carries a factory for one** — `__rt.unscale`'s third argument,
either `() => <zero>` or `null`.

`backend/src/alloc_support.rs` builds it, by decoding an all-zero `Allocation` of the type exactly
the way a `static`'s initializer is decoded. A type the constant reader cannot decode gets `null`,
and a *zeroed* block cast to such a type is then a run time refusal naming it; a block that was
never zeroed is unaffected.

The factory is an arrow rather than a value because it is called once per element: two elements of
an array of structs have to be two objects, or a write through one would be seen through the other.

A cast whose element size does **not** change still has to fill a zeroed block, because a block is
born at `es` 1 and a one byte element is read at the size the block already has. `vec![false; n]` is
the whole of that case, and `__rt.retype` is the member that answers it; it is the identity for
every pointer that is not a fresh zeroed block.

### Freeing

`dealloc` unregisters the block and **poisons** it: the array is emptied, so every stale pointer
into it reads `undefined` rather than the value it used to hold. The block is also remembered as
freed, which turns the next cast through a pointer into it into a refusal that says so.

`realloc` resizes the **same** array. That is the whole point: a JavaScript array grows and shrinks
without moving, so every pointer already derived from the block stays valid and there is nothing to
copy. The new length is in elements at whatever size the block currently holds.

### Run time refusals

Each of these is an exception naming the mistake, not a zombie and not a wrong number, for the same
reason `align_to` is (see "Not supported"): whether a given pointer is a fresh block is a fact about
the value, not about the type, so it cannot be answered when the item is lowered.

- **retyping a block that is no longer fresh** at a different element size
  (`examples/core-tests/guard_heap_retype.rs`). Scalar views over a sealed byte block alias its
  bytes instead of retyping it;
- **a zeroed byte capacity that is not a whole number of elements**, and a byte offset that does
  not land on an element;
- **a zeroed block whose element zero cannot be spelled**
  (`examples/core-tests/guard_zeroed_retype.rs`);
- **a cast through a pointer into a freed block**
  (`examples/core-tests/guard_use_after_free.rs`);
- **reshaping a block that has already been written**, or reshaping one twice at two tail element
  sizes -- the second half of the retype-once invariant above;
- `dealloc` or `realloc` of a pointer that does not name a live block.

### `Box` is its pointer

A `Box<T, A>` whose allocator is zero sized — which is every box in a program with one global
allocator — is represented as the pointer it owns, exactly as a `repr(transparent)` wrapper is
(`value.rs`). `Box<T>` is the slot, `Box<[T]>` is the fat slice, `Box<dyn Trait>` is the fat dyn.
Rust does not call a box transparent, because a box is a language item rather than a library type,
but it holds nothing else.

Two consequences the rest of the backend reads:

- a local of box type whose address is taken is **boxed** like any other local whose value is a JS
  primitive, and dropping one *is* taking its address, so `uses.rs` counts a `Drop` of a place with
  real glue as a bare borrow;
- a `Box<dyn Trait>`'s data half is the *reference* form of the concrete value, which for an
  aggregate is the object and no longer names the allocation. `__rt._put` therefore records where an
  aggregate stored into a heap block lives, so `__rt.box` hands that place back and the box can be
  freed;
- an owned unsized virtual receiver keeps the whole fat-pointer identity until dispatch. Rustc
  spells `Box<dyn FnOnce(Args)>::call_once` as `move *box`, but the virtual ABI still needs
  `meta[slot]` to choose the method and the raw allocation slot as its first argument. The dispatch
  path therefore selects the vtable entry from the fat pointer and passes `__rt.unref(ptr)` for an
  owned receiver. Borrowed virtual receivers continue to pass the reference-form data half.

`examples/core-tests/103_boxed_fn_once.rs` pins owned `FnOnce` receivers with zero, one and two
arguments.

On the JIR optimizer lane, this remains the baseline Box carrier. A private region may instead
select `NativeBox` only when allocation is compiler-owned, pointer/address identity is
unobserved, and exact pointee drop glue is available. Default drop and structural field drop are
semantic operations; a user-defined destructor on the boxed pointee, custom or unknown
allocators, raw pointers, and missing structural glue pin the region to the baseline carrier.
Exact drop glue for fields of a structurally dropped pointee remains legal. Exported Box slots
also remain baseline until the linker has a verified canonical wrapper. Fixtures 89 and 90
protect the native drop cases.

### `String` is `Vec<u8>`

No special representation. A `String`'s bytes live in a heap block of one byte elements, and the
JavaScript string appears only when something derefs it to a `&str` — which `core` does through
`from_utf8_unchecked`, and this backend answers with `__rt.bytes_str`. `bytes_str` has an ASCII fast
path, which is what a program built out of `format!` actually hits.

### The string sink

Building a `String` a character at a time through the heap works, but it spends an array element per
byte and a UTF-8 encode per push. A **sink** is the direct route: a host side string builder that a
`fmt::Write` implementation pushes into, and one JavaScript string at the end. The handle is an
index rather than the builder itself, so the whole surface is `usize`-shaped and an `extern "C"`
declaration can name it:

```rust
unsafe extern "C" {
    fn sb_new() -> usize;
    fn sb_push(handle: usize, s: &str);
    fn sb_push_char(handle: usize, c: char);
    fn sb_take(handle: usize) -> &'static str;
}
```

`sb_take` empties the sink, so a handle is reusable. The `'static` is a convenient fiction: a
JavaScript string is a value, owned by nobody. `examples/core-tests/prelude.rs` wraps the four in
`JsStr`, and `23_format.rs` is the golden.

### Host collections

`std`'s `HashMap` is `hashbrown`, and `hashbrown` cannot be compiled by this backend. Its group scan
reads sixteen control bytes as one wide integer and punnes the result back to a bitmask, and its one
allocation holds control bytes and table entries at **two element granularities**. A heap block here
is a JavaScript array of one element type and is retyped exactly once, so both are run time refusals
rather than slow paths (see "Retyping" and "Run time refusals" above). No amount of work on the
allocation model changes that: it is the same reinterpretation `guard_heap_retype.rs` pins.

The host already has a hash table. `topcoat-js` is the crate that hands it to Rust as
`topcoat_js::collections::{HashMap, HashSet}`, and `examples/core-tests/30_hashmap.rs` is the
golden.

**A `HashMap<K, V>` IS a host `Map`.** `repr(transparent)` over a one word handle, exactly as
`view_abi::Node` is over a DOM node: the JavaScript value the backend keeps in a local of that type
is the `Map` object itself. There is no handle table, no side allocation and no `Drop`; nothing
frees a map but the engine's collector, which is what makes moving one into a struct, into a
closure or through a function free and identity preserving. A `HashSet<T>` is a `HashMap<T, ()>`
under the same wrapper.

**A key is a JavaScript value a `Map` compares by value.** A `Map` compares with SameValueZero,
which is by value for a number, a BigInt, a boolean and a string, and object *identity* for
everything else. So the key classes are exactly:

| Rust | crosses as | note |
|---|---|---|
| `i8`..`i32`, `u8`..`u32`, `isize`, `usize` | a number | |
| `i64`, `u64`, `i128`, `u128` | a **BigInt** | equal by value; `5n` is not `5`, but one map has one key type |
| `bool` | a boolean | |
| `char` | a number | the code point, as everywhere else in the value model |
| `str`, `&str`, `String` | a string | one class, so the three are interchangeable |

Floats are excluded, for the reason `core` gives them no `Eq`. Everything else is excluded because
the entry would be filed under the object that carried the key and never found again by an equal one
built later, which is a silent wrong answer with nothing reported anywhere.

**The refusal is a type error, twice over.** `topcoat_js::collections::MapKey` is sealed, so an
aggregate key is rejected at the call site with the key type named, before any code is generated;
`examples/core-tests/guard_map_key.rs` pins that message. `backend/src/map.rs` carries the same rule
again as a post-monomorphization check on the marker's key argument, which is unreachable through
that crate and exists for a marker any other crate could call.

**`MapKey::Class` is what a key compares as**, and a lookup takes the class rather than the key
type: `String`, `&str` and `str` all have `Class = str`, and every other key is its own class. So a
`HashMap<String, V>` is read with `map.get("name")` and never needs a `String` built to ask a
question. A `String` key is decoded to a host string on the way in (one `__rt.bytes_str`), so the key
the map holds and the `String` that carried it part ways immediately: dropping the `String` frees its
heap block and leaves the map's key standing.

**A stored value is the `Option<V>` holding it, built on the Rust side.** Nothing hands the host a
bare `V`. This is what keeps the shim from ever learning what a Rust value looks like, and it falls
out of "Enums": an enum with a payload-carrying variant is always an object and always
[indirect](#value-representation), so `map.get(k)` of a present key is always an object, `&Option<V>`
is always that object, and the marker lowering is a plain call with no case analysis on `V`. The
three things that follow:

- `get`/`get_mut` are `Option::as_ref`/`as_mut`, so `&V` and `&mut V` come out in whichever of the
  two reference shapes `V` needs -- the object for an aggregate, a `{ buf, off }` slot naming the
  `_0` field for a primitive -- built by the value model rather than by the shim. A write through
  `get_mut` names the place inside the host object, so it is what the next `get` reads;
- `insert`'s old-value answer is `Option::replace` and `remove`'s is `Option::take`: safe code
  writing through an ordinary `&mut`, rather than a `ptr::read` a caller has to promise is a move;
- an absent key is never `undefined` reaching Rust. Every operation tests `map.has(k)` first.

**Iteration is over a snapshot.** `iter()` takes the keys once, as a fresh host array, and the
`__rt.map_keys` that builds it returns a **fat slice** (`{ buf, off: 0, len }`), which IS a `&[K]` in
the value model. So walking it is ordinary Rust indexing and the backend needs no arm for it. The
map cannot change while the iterator is alive, because the iterator borrows it; the snapshot is
therefore only visible as a cost, one array of `n` keys and one lookup per key. Order is the host's
insertion order, which `Map` guarantees.

#### The markers

Monomorphic operations are plain `extern "C"` declarations and reach `__rt` with no help from the
backend at all ("Calls to foreign items"). Only the operations generic over the key or the value
need one, because rustc has no generic foreign declaration, and those are markers in the same style
as `view-abi`'s: an ordinary Rust `fn` with an unreachable body, recognized by its `link_section`
before the callee resolves. The prefix is `rcgjs.map.` and `backend/src/map.rs` is the whole
lowering.

| section | lowering |
|---|---|
| `rcgjs.map.has` | `__rt.map_has(m, k)` |
| `rcgjs.map.set` | `__rt.map_set(m, k, v)` |
| `rcgjs.map.get` | `__rt.map_get(m, k)` |
| `rcgjs.map.getmut` | `__rt.map_get(m, k)`, the same read |
| `rcgjs.map.del` | `__rt.map_del(m, k)` |
| `rcgjs.map.keys` | `__rt.map_keys(m)` |

Two decisions are made there and only two. The **key crosses by reference**, because that is the one
shape covering both classes -- a `&str` already IS the host string and a `&u32` is a slot -- so a
slot shaped key argument is read through with `ptr::slot_element`, the same question
`text_conversion` asks of a `view_abi::text` argument. And the **key class is checked** after
monomorphization, reported as a zombie at the call site naming the type. A marker section the backend
does not know is a zombie naming the section, exactly as an unknown `rcgjs.tc.` one is; adding a
marker here bumps no version, for the reason the template markers give.

#### Runtime shim additions

```js
__rt.map_new()            // new Map(); the map IS the value
__rt.map_len(m)           // m.size
__rt.map_clear(m)         // m.clear()
__rt.map_has(m, k)        // m.has(k)
__rt.map_set(m, k, v)     // m.set(k, v), where v is the Option object Rust built
__rt.map_get(m, k)        // m.get(k), only ever called for a key map_has answered for
__rt.map_del(m, k)        // m.delete(k)
__rt.map_keys(m)          // { buf: [...m.keys()], off: 0, len }: a snapshot, as a `&[K]`
```

`scripts/make-esm-shim.mjs` needs no list to keep in step: it evaluates the shim and exports the keys
of the `__rt` it built, so these are in the ES module form by construction, and `jsc-build`'s
`shimcheck` reads that module's own exports.

#### Not supported

- **An `entry` API.** `or_insert_with` and friends would need a place a closure can be called into
  and a key held across it; the four call shape (`has`, `get`, `set`, `del`) has no room for it
  without a handle to a slot the map may rehash under. Deferred, not refused.
- **A key that is not one of the classes above**, including a float, a tuple, an array and any
  struct or enum. Sealed trait, plus the backend check.
- **`&mut` aliasing across a lookup.** Two `get_mut` calls for one key hand out two references to
  one place, and only the borrow checker stops that: the markers return a reference at a lifetime
  the caller picks, so `topcoat-js` narrows it to a borrow of the map at every call site and the
  markers are `unsafe fn` to say so.

### The allocator shim

`rustc_codegen_ssa` asks the backend to synthesize the `__rust_*` symbols `library/alloc` only
declares. `backend/src/alloc_support.rs` answers with real bodies over the heap above, following
`rustc_codegen_cranelift`'s `src/allocator.rs`: one function per `AllocatorMethod`, named
`mangle_internal_symbol(global_fn_name(name))`, parameters in `inputs` order with a `Layout` split
into its size and its alignment. cg_clif forwards every method to the `__rdl_*` wrapper `std`
defines; there is no `std` here, so the four methods rustc marks with a `SpecialAllocatorMethod` are
answered directly and only an unrecognized method still forwards.

**The alignment arrives as a plain number.** `Layout::align` is a `core::mem::Alignment`, which is a
`repr(transparent)` struct over a fieldless `repr(usize)` enum, and such an enum *is* its
discriminant (see "Enums"). So the parameter already holds the number `__rt` wants and there is
nothing to unwrap, which is also what makes a `transmute` of an `Alignment` to a `usize` the
identity.

`patches/0002-alloc-default-lib-allocator.patch` marks `alloc` as `#![default_lib_allocator]`, which
is what makes `AllocatorKind::Default` the kind a program using `alloc` gets, and so what lets an
ordinary `#![no_std]` crate allocate with no `#[global_allocator]` of its own.

An allocator shim is **not** a dead code elimination root, so a program that never allocates carries
none of it.

### Runtime shim additions

```js
__rt.alloc(size, align)                 // __rust_alloc: a fresh byte granular block
__rt.alloc_zeroed(size, align)          // __rust_alloc_zeroed: the same, filled with zero bytes
__rt.dealloc(p, size, align)            // __rust_dealloc: unregister and poison
__rt.realloc(p, size, align, new_size)  // __rust_realloc: the same array, resized
__rt.alloc_error(size, align)           // __rust_alloc_error_handler: diverging
__rt.unscale(p, size, mk)               // gains the zero element factory (see above)
__rt.retype(p, size, mk)                // the fill half, for a cast that changes no size
__rt.retype_rc(p, header_mk, es, len)   // the reshape into a header plus an unsized tail
__rt.unref(p)                           // the raw pointer a `&dyn Trait`'s data half denotes
__rt.unwindow_slice(p, len)             // the fat slice a raw `&[E; N] -> &[E]` produces
__rt._put(buf, off, value)              // the one element store, now also emitted by the backend
__rt.sb_new()                           // a string sink handle
__rt.sb_push(handle, s)
__rt.sb_push_char(handle, c)
__rt.sb_take(handle)                    // the finished string; the handle is reusable
```

`__rt._put` is what a write of a whole aggregate through a pointer becomes. It overwrites the object
in place wherever there is one, so every alias still sees the write, and stores where there is not —
which is what a fresh heap block needs, since `Box::new` writes the whole pointee into one.

## Modules (stage 3)

`-Cllvm-args=js-modules=esm` makes the program an ES module. The default is `script`, which is
everything above: a file that expects `__rt` to be a global and leaves its exported items as
globals for the host to call.

A module differs from a script in exactly two lines of the file.

```js
import * as __rt from "./shim.js";   // first, before every declaration
// ... every item, byte for byte what the script build prints ...
export { counter_clicked, rust_entry };   // last, after minification
```

**The `__rt` import.** Synthesized by the link step, not by codegen. It is deliberately not a
dead-code-elimination root: `refs` already reports `__rt` for every `__rt.foo(...)` call an item
makes, so the import is reachable exactly when something uses the shim and is dropped when nothing
does (`examples/emit/16_esm_no_shim.rs` is the fixture that pins the drop). `js-shim-module`
decides the specifier; it defaults to `./shim.js`.

**Imports are items.** `ItemKind::Import` is an item like any other, so it deduplicates, orders and
survives dead code elimination through the machinery that was already there. One item binds one
name and **the item's name is that binding**, which is what makes reachability work: an import is
kept when another item mentions what it binds. Two crates that import the same name from the same
module produce one item with one text, which the link step collapses silently; the same name from
two different modules is the existing `two different definitions of ...` error. An import item's
`debug_path` is the module specifier it imports from.

**Order.** Imports come first, sorted by `(module, binding)`; then the module level bindings in
dependency order; then everything else. That is one new tier on top of the order stage 1 had.

**Names.** An imported binding is never renamed, by either half of `js-minify`: the renamer
rewrites text, and renaming the binding inside an `import` would import a name the module does not
export. `_$` is reserved for the DOM runtime's bindings (`_$insert`, `_$template`), and no
generated local ever starts with it, so an imported binding cannot be shadowed either.
`js-dom-module` decides where those come from and defaults to `topcoat-dom`.

**The export clause** names the `#[no_mangle]`/`#[export_name]` items of the crate being linked, in
sorted order, in one trailing statement. One clause rather than an `export` keyword on each
declaration, and it is written after minification, so that no item's text depends on the module
mode. A dependency's exports are not in it, for the same reason they are not roots. The
`//# sourceMappingURL` line, when there is one, still comes last.

### Why `js-modules` is not emit-affecting

**An item's own JavaScript is byte for byte identical in the two modes.** Everything a module adds
is synthesized at link time, out of items the codegen step never saw. So a crate compiled with
`js-modules=esm` links against a sysroot built without it, and switching modes never costs a
sysroot rebuild — `scripts/test.sh` filters the module options out of the comparison it makes
against `build/sysroot/.js-args` for exactly this reason.

`scripts/module-test.sh` is what holds the invariant: it compiles every `examples/emit` fixture
both ways, checks the shape (imports first, one export clause, last line), strips those two lines
and diffs the rest against the script build byte for byte, then runs both and diffs what they
printed. If it ever fails, `js-modules` has become emit-affecting and the sysroot rule above is
wrong.

### The shim has two forms, from one source

`runtime/shim.js` stays a plain script: it assigns `globalThis.__rt`, it is concatenated ahead of a
script mode program, and pages load it with a bare `<script src>` and extend the object afterwards
(`demo/index.html` and `demo-app/src/glue.js` both do). A module cannot use that file — a
namespace import needs named exports — and one file cannot be both, because `export` is a syntax
error in a script.

So there is one source and one derived artifact, never two hand written copies.
`scripts/make-esm-shim.mjs` writes the module form: the shim's own text, verbatim, followed by an
export clause. The names in that clause are not a list anybody maintains — the script is evaluated
and the clause is written out of the keys of the `__rt` it actually built, so a member added to the
shim is exported by the next run and the two forms cannot drift. `scripts/run-esm.sh` derives it
into `build/esm/shim.js`, beside the program that imports it.

One consequence for a host: in script mode extra foreign items are added by assigning to
`globalThis.__rt` after the shim loads, and a module namespace object cannot be extended that way.
A module program's host passes `js-shim-module` a module of its own that re-exports the shim plus
whatever else the crate declares.

### Object file format version

An object file carries a `u16` format version. postcard is not self describing, so a table written
in an older shape does not fail to parse — it parses into something else, and the program that
comes out is wrong rather than rejected. A mismatch is an error naming the fix
(`scripts/build_sysroot.sh --clean`). Bump `FORMAT_VERSION` in `rcgjs-object/src/object.rs`
whenever the shape changes -- and also when the **JavaScript inside it** changes incompatibly,
which is the same failure one level up: an object holding the old enum encoding would link cleanly
against new objects and produce a program that is wrong rather than rejected.

Format 20 (Phase 1 W2) replaced the trailing `//# rcgjs:` text footer with a binary member: the
printed baseline JavaScript still comes first as a plain UTF-8 text section, then a `RCGJS\0`
member holding a header and six checksummed sections (interns, the eagerly read summary index,
content-addressed deflated JIR-S bodies, the per-item baseline records, origins, and the frozen v1
optimizer's metadata), then a fixed 16-byte trailer so a reader finds the member without scanning
the text. The header pins the JIR schema, the rustc that produced the object, its data layout and
the emit-affecting options. `rcgjs-object/src/object.rs` is the specification.

Format 27 and JIR schema 15 carry source coroutine control-flow and suspension facts in the intern section. The pinned rustc backend wraps `mir_drops_elaborated_and_const_checked`, borrows `mir_promoted` before delegating to rustc, and stores owned yield locations, bare resume locals, and the complete source block graph. Each block records its terminator class and ordered normal, unwind, resume, cancellation, or imaginary successors. Bare drop locals and direct source callee identities are retained before MIR inlining; projected drop places and dynamic callees remain unavailable. The producer checks the complete ordered target sequence against rustc's successor iterator. The reader validates edge targets and terminator arity, requires exactly one matching yield record per yield block, and checks its resume/cancellation edges. These indices belong to the generic analysis body, including unreachable blocks. They are not indices into the transformed JIR body or proof of dynamic suspension counts. Local coroutine queries are requested before codegen units serialize their metadata, including generic-only libraries. The object reader retains those facts independently of emitted items and rejects invalid indices. The linker merges them by stable definition identity, resolves object-local source positions, and reports conflicting captures explicitly.

The compiler session retains each captured coroutine's promoted MIR as owned tokens through rustc's encoder and reconstructs it through rustc's decoder when codegen requests source facts. This preserves instruction operands, types, source scopes and cleanup edges after the original provider consumes the body, without retaining borrowed MIR or modifying rustc's passes. Session-local span, definition and allocation IDs make these snapshots unsuitable for object files or reuse in another compiler session. Only the existing source-fact tables cross that boundary. Self-check mode round-trips ordinary bodies after provider delegation too, compares the full body with only recomputable CFG caches cleared, and re-encodes the result. The suspension-facts harness requires these checks. Retaining source MIR does not elaborate its drops, emit exceptional cleanup or provide executable generic bodies to downstream crates.

The producer also follows existing debug-place records through the pinned analysis-to-runtime passes, which preserve their order. It retains each record's original bare local identity before delegation, checks record count, name and source scope afterward, then resolves the transformed whole-field projection through rustc's runtime coroutine layout. A generated Pin alias must have one direct assignment from the coroutine argument. This does not add or alter MIR debug records. The optional `runtime_frame` table records saved-field indices, state membership and proven source-local correspondences. Empty source lists remain explicit for unnamed, projected or transformed-away records, including compiler-generated cleanup state. A changed record identity makes the table unavailable. The linker separately checks its shape against the monomorphic coroutine type. This maps whole source locals to runtime fields; it does not map source yield blocks to runtime states, reconstruct initialized cleanup or select continuations.

The schema-8 `async` report keeps source yield sites separate from the estimated post-transform suspension states. An unavailable capture stays unavailable, including when an incremental query bypasses the producer hook. The backend keeps no MIR references in its session cache and supports one compiler session per process, as its global options do. Each codegen unit records the source queries as dependencies so edits to a generic-only library invalidate its saved metadata. The report derives normal-flow segments from block zero and every resume destination, stopping at yields. Segments may overlap and contain loops; imaginary borrow-check edges are excluded and unwind exits are reported separately. A resume destination from an unreachable yield still has a segment, so segment existence does not prove entry reachability. Potential async-drop cancellation edges are preserved on `Drop` terminators. A source yield count of zero does not prove that driving the future cannot suspend. The graph does not contain executable instruction operands, complete call effects, or a runtime drop plan. These facts alone never authorize native continuation lowering. Await relationships, wake sources, source-to-state mapping, live storage, initialized cleanup, and driver/cancellation legality require separate proofs. `scripts/suspension-facts-test.sh` checks local and generic-library captures, source lines, zero fallback, runtime results, off/report JavaScript identity, and cold/warm/edited incremental builds. Its independent `source-cfg-mir.py` check compares every block and edge with the pinned rustc analysis MIR dumps in cold and edited builds. The same checker independently reads runtime saved-field layouts and debug places, and rejects swapped source mappings and missing state fields.

Each yield can also carry `liveness` with sorted source-local indices for retained values, retained locals previously borrowed, and live storage. The producer reuses the pinned `rustc_mir_dataflow` analyses and validates the retained union, field types, source identities, per-yield field order and source positions against `mir_coroutine_witnesses`. No source-position matching is used to infer local identity. Missing witnesses remain unavailable. The object reader rejects duplicate, out-of-range, self-retention and inconsistent borrowed-local records. These are promoted-MIR facts, not indices into the optimized runtime frame, initialized drop flags, or an executable continuation plan.

The liveness fixture distinguishes a retained borrow, a consumed guard, a held guard and values live at different yields. Independent MIR checks resolve the authored local names and reject omitted live/borrowed records. Cold/warm/edited builds also replace a borrow with a value copy while keeping the yield count unchanged, verifying that incremental metadata tracks the changed retained state. Runtime-frame correspondence, executable source operands and state-specific cleanup remain separate requirements.

The optional `initialization` table records conservative promoted-MIR move-path state before and after each terminator's primary effect. The producer reuses rustc's default maybe-initialized and maybe-uninitialized analyses over all tracked types. A path present in both sets is conditional; an aggregate's children must be considered separately after partial moves. Paths preserve their source local, parent and projection indices. Type-changing projections without generic type identity remain unavailable. Unreachable points carry no state. The object reader checks path ancestry, local and path indices, sorted unique sets and block coverage. These facts precede rustc's later drop-elaboration refinements and are not runtime drop flags. After a call's primary effect, moved arguments have left the caller but the return destination is not yet initialized: that happens only on the normal-return edge.

The initialization fixture observes uninitialized and initialized locals, a partially moved tuple, conditional initialization across a real suspension, and call argument/destination state. Independent textual MIR probes locate the authored locals and calls. A conditional-to-definite initialization edit preserves the yield count while changing the serialized state, and four deliberately corrupted reports must fail the probe checks. Executable operands, runtime state correspondence and cleanup emission remain open.

### Experimental ready Promise boundaries

`js-jir-ready-futures=on` is a link-only opt-in for ordinary `future_to_promise` boundaries under the JIR `future-poller` pass. It is off by default and in every release profile. The linker requires a compiler-generated coroutine with no source yields or saved suspension state, plain captures without references or destructors, no source-local cleanup, an unused poll context, and only proven typed extern calls. Source callee identities also reject ordinary helpers hidden by MIR inlining. The positional producer/linker manifest is version 4. Custom pollers, awaits, retained borrows, and unsupported cleanup use the established executor.

The selected `host_future_ready` driver preserves the existing poll and drop callbacks and passes a valid no-op waker. It creates a fresh Promise, drives synchronously, reads the ready result before dropping the future, and clears its retained frame before Promise adoption. Poll and drop exceptions retain their identity, with a drop exception taking priority. This removes scheduler flags, queued wake handling, and scheduling closures; it does not remove the Rust poll state machine, Context construction, or callback glue. It adds no Promise job boundary before first progress. Ordinary future construction stays inert until driven.

Selection requires all analyzed ordinary Promise boundaries to be eligible, so a mixed program does not intentionally add a second driver facility. Any final lowering refusal while the ready plan is selected retries the whole program with that plan disabled. Report mode records `would_select` while retaining off-identical JavaScript. Async report schema 8 exposes `ready_boundaries` separately from the unselected suspension graphs. Rows explicitly leave `package_profitability_proven` false: the override is experimental, and complete-package economics and protected runtime remain external admission requirements.

`scripts/promise-ready-test.sh` verifies compiled transition traces, inert construction, thenable adoption, rejection identity, reentrant driving, decline controls, source-call checks after MIR inlining, and injected whole-plan rollback. `scripts/runtime-ready.test.mjs` pins callback/drop exception priority and result access order in both full and extracted runtimes. `scripts/promise-ready-package.mjs <build-directory>` compares the complete minified runtime closure against the same fixture with the option off. `scripts/promise-ready-browser.mjs <build-directory> <output-directory>` runs the compiled fixtures, decline controls, rollback and complete packages in Chrome, Firefox and Safari. These checks do not qualify general suspending continuations, initialized-state cleanup, owner cancellation, or automatic release-policy selection.

### The link step is a separate program, and the lane is its decision (v2)

The dylib no longer links anything itself. It writes object files and execs its sibling binary
`jsc-link` with a manifest; the graph walk, deduplication, chunking, the source map, the `.d.ts`,
the report and the optimizer passes all live there. `scripts/build.sh` builds both and prints
both paths, and `JSC_LINK_BIN` overrides where the dylib looks. `jsc-link/README.md` is the
specification of the manifest and of the lanes.

Because an object file carries both v1's printed JavaScript and the item's JIR-S body,
`-Cllvm-args=js-lane=baseline|jir|shadow` chooses which spelling each item is printed from **at
link time**: it is not emit-affecting, it does not enter the sysroot cache key, and one set of
objects serves every lane. `baseline` is the default and is byte-identical to v1;
`jir` prints the JIR lowering wherever it succeeded and v1's text for the rest, beside a
`<out>.lane.json` sidecar; `shadow` prints both programs from one link so they can be diffed.
`js-lane=jir` composes with `js-opt=on`: the pair was refused through Phase 2, because v1's text
optimizer rewrote baseline JavaScript the lane does not print, and Phase 3 lifted the refusal by
re-landing the rewrites as JIR passes (`jsc-link/src/opt/`). Phase 5 (W5h) then deleted the v1
text optimizer outright: on the baseline lane `js-opt=on` now means the producer-baked passes plus
link-dedup, with no link-time text rewriting, and `on` output is byte-identical to `off` plus
dedup. `-Cllvm-args=js-jir-flat-abi=on|off` (default
on, link-only) turns the lane's flat direct-call ABI off for bisecting.
`-Cllvm-args=js-jir-typed-storage=on|off` (default off, also link-only) enables the experimental
exact/reserved numeric storage path. It has no effect unless the lane is `jir` and the optimizer is
`on`, and it does not require a distinct sysroot.

The flat ABI covers eligible private direct-call parameters and narrow field-only product
results. Result lane 0 is returned normally; additional lanes use program-wide collision-safe
`$rN` spill bindings and must be consumed immediately by the direct caller. Caller and callee
rewrite transactionally, and any unsupported export, indirect use, suspension, or lowering retry
restores the canonical materialized ABI. Fixture 91 protects the result path. Interprocedural
projection forwarding may reuse call arguments only for an immediately adjacent same-block copy;
fixture 92 protects the mutation boundary.

The typed-storage override does not change the semantic `NativeVec` carrier. For an eligible
private `Vec<T>::with_capacity`, it allocates the buffer as one `Uint8Array`, `Int8Array`,
`Uint16Array`, `Int16Array`, `Uint32Array`, `Int32Array`, `Float32Array`, or `Float64Array` at the
greater of a constant initial capacity and a proven maximum logical length. The 32-bit target's
`usize` and `isize` use the corresponding 32-bit arrays. The vector still carries independent
`len` and logical `cap` fields; TypedArray length is never exposed as Rust capacity.

The proof follows only a unique acyclic `Goto` path and applies integer-capacity casts with exact
source/target widths and signedness. It permits private push, clear, length, emptiness,
indexing/mutation, POD drop, and an immutable slice consumed only by a selected scalar
`buf, off, len` direct-call ABI. A branch or unknown mutation loses the length proof. Dropful,
zero-sized, unsized, or unsupported elements; custom or unknown allocators; capacity/layout,
identity, address, or raw-parts observation; retained or mutable views; owned copies;
remove/extend emission; and unadapted call, host, indirect, vtable, export, store, or return
boundaries decline the complete value-local choice.

Current schema-15 lane reports publish `constant_initial_capacity`, `proven_maximum_length`,
`proven_exact_final_length`, `selected`, and an exact terminal reason for every candidate.
`storage_mode` records whether the executable override was active, while selected rows record what
actually changed. With the option absent, reporting remains observational and emission is
unchanged. Fixture 104 and `scripts/storage-plan-test.sh` are the executable contract.

Finished-AST SROA on a JIR-lowered declaration may forward a direct compiler-local alias only when
the JIR SSA definition dominates it and the source slot is never replaced: an unassigned parameter
or a local with exactly one whole write. The alias must itself have one source. A field alias keeps
the narrower rule: its root is a fresh object literal and its final field is never assigned. The
rewrite redirects reads to the already-evaluated source slot rather than moving the source
expression, so object identity and mutation remain visible; reassigned sources, replaced-field
snapshots, shadowed names, and verbatim JavaScript decline.

JIR schema 9 appends one exact standard-iterator identity to `TypeMeta`. `SliceIter` and
`Enumerate` use existing rustc diagnostic items; `patches/0013-iterator-diagnostic-items.patch`
marks `Take` and `Skip`. The producer records only those compiler identities. A path, source span,
debug name, or emitted JavaScript name is never semantic evidence for iterator lowering.

The finished-JIR `slice-iter` pass covers exactly
`Enumerate<Take<SliceIter<u8>>>` and `Enumerate<Skip<SliceIter<u8>>>`. The complete construction,
source/end pointer relation, slice metadata length, direct or fresh mutable-borrow driver, loop SCC,
backedges, and common exit must match. Adapter values may not escape. Blocks between a loop exit
and its one parameter-free common exit may only read, project, borrow, cast, or discardably update
the dead adapter state. Calls, arithmetic, unrelated stores, observed remaining length, other
element types, and other adapter orders decline the rewrite.

The replacement carries one checked `usize` cursor. `Take` tests both source length and limit;
`Skip` begins at the limit and subtracts it for the enumerate index. The indexed byte borrow and
cursor increment remain explicit JIR operations. Existing DCE removes the unreachable adapter
graph. Before later slice forwarding, a `SliceIteratorCursor` witness independently checks the
exact adapter, source, length, limit, cursor, guards, enumerate index, borrow, preheader, backedges,
body, and exit for every selected loop. Complete rewrite SCCs and their preheaders must be pairwise
disjoint. Failure preserves the original item with `proof.witness`. Lane-report schema 15 publishes
global and per-item `slice_iter.would_lower` and `slice_iter.final_emitted`; report mode and witness
fallback always report zero final cursors. Fixture 105 and `scripts/slice-iterator-test.sh` define
the positive, explicit decline, early-break, and targeted-witness-fault contract.

Lowering may inline a synthetic slot view only while its root is pure. An effectful expression that
returns a slot is materialized before dereference, store-through, or the first borrow/raw view; this
preserves Rust's one-evaluation rule. Pure field views stay visible to the alias planner instead of
being hidden behind a temporary, so later stores still invalidate their reads. The JIR lowering
tests pin both sides of that rule.

## Chunks (stage 4)

`-Cllvm-args=js-chunk=<name>:<entry>`, repeatable, splits the program into one file per entry point.
`-Cllvm-args=js-chunk-shared=<name>` names the file holding what two or more of them reach; it
defaults to `shared`. Each chunk is written as `<name>.js` beside the output path, with its source
map as `<name>.js.map`, and the shared file appears only when there is something to share: its
absence is the answer "these chunks had nothing in common", not a missing output.

Chunks need `js-modules=esm`, because a chunk reaches the others by `import` and a script cannot.
Asking for them without it is an error rather than a silently unsplit program.

**The output path is unaffected.** A split is an *extra* emission: the file the compiler was asked
for still holds the whole program, byte for byte what it holds with no chunk asked for. That is what
keeps `--emit=obj` runnable, leaves every existing golden alone, and lets a build tool ask for chunks
before knowing whether there is anything to split. `examples/emit/20_chunk_one.rs` pins the
degenerate case: one chunk over the program's only root is the whole program back, differing only in
the header comment that names the file.

### Who owns what

Reachability is a pure walk over the item table, so a chunk is one more walk from one more root.
Every name reached gets exactly one owner:

1. a name that is some chunk's entry point belongs to that chunk, so `<name>.js` always exports the
   entry it was asked for;
2. a name only one chunk reaches belongs to that chunk;
3. a name two or more chunks reach belongs to the shared chunk;
4. an `ItemKind::Import` is the exception: it is **replicated** into every chunk that reaches it
   rather than owned by one. An import *declares* a binding the module has to have, so moving one
   into the shared chunk would leave a chunk calling `__rt.foo(..)` with no `__rt`.

A chunk then imports every name its own items mention that another chunk owns, and the owner exports
it. The shared chunk is closed under references -- everything an item two chunks reach can itself
reach is also reached by both -- so it never imports from a chunk and the module graph has no cycles.
Rule 1 is applied last, so an entry point another chunk also reaches stays in its own chunk and the
other chunk imports it: island-to-island falls out of the same mechanism as island-to-shared instead
of being a broken edge case. (The shared chunk may then import an entry point back, but a chunk
whose entry another file names owns nothing else: everything that entry reaches is reached twice.)

**Around a cycle of imports, only functions.** When files import each other, which one is
evaluated first depends on which file the host loads first, so a module level initializer that
reads a binding imported around that cycle reads it uninitialized under some load order. Function
declarations exist before any module is evaluated, so importing a function around a cycle is
always safe. The link step therefore refuses a split in which a module level binding of one file
names a binding of another file that imports it back, directly or through other files. With
functions as entry points this never happens: a file another file imports from holds nothing but
its entry. A `static` may be an entry point (it is exported, not called) as long as its imports run
one way, as in `265_chunk_static_entries`: it may name shared bindings when no shared binding names
it, or be named by shared bindings when it names none of theirs. `266_chunk_static_entry_refusals`
pins the refusal.

**A unit declares what it owns, and only that.** Each file's module level bindings are ordered over
the names that file owns: a binding it owns that names a binding another file owns comes after the
`import` of that name and does not declare it again. (Following the name into the file, as the
whole-program order does, declared it twice, a `SyntaxError` before the module ran.) A cycle of
statics is reached as a whole by every chunk that reaches one member, so it has one owner and is
initialized there in two steps (see "Statics that refer to each other"). The one way to split a
cycle, naming one of its statics as a chunk's entry point, is a link error with its own message.
`scripts/chunk-test.sh` runs the core fixtures that carry a `.chunks` file split
(`262_chunk_owned_statics` to `266_chunk_static_entry_refusals`), including the splits that have to
be refused, and `scripts/check.sh` runs it.

### The cross-chunk specifier is relative

```js
import { lib$impl_1_get$h22b2.., lib$impl_1_set$h75b7.. } from "./shared.js";
```

**Relative, not the bare chunk name, and that is a decision rather than an accident.** Every chunk is
published into one directory, so a sibling path is correct wherever the files are served from and
needs no import map entry to resolve -- in a browser and under node alike. A bare specifier would
need one, and the prefix that map is written with belongs to the build tool: it is configurable
there, and the compiler is never told what it is. A chunk therefore cannot spell it.

The consequence for a build tool: with chunks in a subdirectory of where the shim is published,
`js-shim-module` must be an absolute URL, a bare specifier, or a relative path that accounts for the
extra level. The default `./shim.js` resolves inside the chunk directory.

### Minification across chunks

The item rename is computed **once** over the concatenation of every chunk and applied to each, so a
name that crosses a file boundary is spelled the same way on both sides of the boundary. Both ends
are ordinary text -- the `import { .. }` clause and the `export { .. }` clause are scanned like any
item's code -- so nothing special is needed to keep them in step. The exported names of the program
itself are unaffected: those carry a fixed name, which minification never touches.

An unsplit program is renamed exactly as it was before chunks existed, because the corpus is then
that one program.

### Zombies name the chunk

With chunks, a zombie is reported by the first chunk that reaches it, and the report carries the
chunk's name beside the chain of items that made it reachable. So the question a split program
answers is "which entry point cannot be built", which is the useful one. Each is reported once
however many chunks reach it.

## Coroutines (stage 4)

An `async fn` is a coroutine, and after rustc's `StateTransform` pass a coroutine is **an enum in
everything but its type kind**: its layout is `Variants::Multiple` with a tag, its states are
variants, and the locals it holds across a suspension are per-state fields. So it needs no machinery
of its own, only answers from the value model.

Its JavaScript value is an object whose `TAG` is the state's **index** (`EnumRepr::State`), because
its states have no names to spell. Always an object and never the bare number: a coroutine holds its
upvars beside the tag from the moment it is built, and it is only ever reached through a `&mut`
(`Future::poll` takes `Pin<&mut Self>`), so a value with no identity to write through would be wrong.
The index is also the discriminant `SwitchInt` compares against, which is why the numeric decision
tree drives the resume unchanged.

**Two field spaces share the one object, and they must not collide.** With no state downcast a field
is an **upvar** -- the layout's prefix, exactly a closure's captures, keyed by index. Inside a state
it is a **saved local**, keyed `$s<n>` by the `CoroutineSavedLocal` the layout names it by and *not*
by the per-state field index: one local can sit at different indices in two states that both hold
it, so keying by the index would make one local read as another after a resume. That is a silent
wrong answer, which is what the separate key space buys.

Building one puts it in its unresumed state holding its upvars. Changing its state moves the tag and
leaves the saved locals where they are, because MIR writes the new state's fields as separate
statements and says nothing about the old ones.

### The executor

The `view-async` crate drives Rust futures under a captured reactive owner. With optimization disabled, a safe Rust `Rc<Task>` holds the pinned future and a callback-backed waker. Optimized builds pass the concrete Rust poll and drop functions to `__rt.host_spawn`, which holds the scheduling state in JavaScript. Both paths defer the first poll to a microtask, coalesce wakes, and preserve Rust's future state and drop glue.

The Promise bridge uses two primitives. `__rt.microtask(f)` is `queueMicrotask`. `__rt.settled(v, f)`
calls `f(x, true)` when `v` resolves and `f(e, false)` when it rejects, and queues `f(v, true)` for
a value that is not a thenable at all. Two decisions there: the thenable test is
`typeof v.then === "function"` and not upstream's `"then" in v` property test (CONTRACT-DOM 14.9),
because this one refuses to call what is not callable; and a rejection is delivered as a VALUE with
`ok === false` rather than left as a rejection, because the pinned runtime's error handling is
entirely synchronous and a rejection reaches none of it (CONTRACT-DOM 14.4).

The bridge reads `then` once and invokes that retrieved callable with the original value as its receiver. Getter, call, and synchronous callback exceptions propagate unchanged. Reading a callable's own `call` property is unnecessary and may itself invoke a getter or proxy trap. Non-callable values retain the deferred successful callback.

Both `topcoat_js::await_promise` and `view_async::await_promise` catch an exception from that subscription call and return `Poll::Ready(Err(error))` in the first poll. The awaiting Rust body can then follow its normal error path and drop initialized locals. A synchronous settlement callback followed by a throw still returns the thrown error; queued or later callbacks cannot resume a completed driver. Successful subscription keeps the existing later-poll scheduling. This boundary does not catch exceptions from unrelated Rust or host calls or provide Rust unwinding.

`view_abi::try_host` exposes the existing `rcgjs.host.try` operation for a closure returning unit and a typed error. As with `on_settled`, the error type is the caller's claim about the host value. `view_async` uses this boundary without leaving its captured owner. The subscription suite covers host values, strings, numbers, and unit, including deferred first polling, exact microtask order, initialized-local cleanup, rejection identity, and late callbacks after a throw.

Both safe Rust task drivers catch host exceptions around polling while keeping the outer future-slot borrow outside the catch boundary. Terminal state is marked and the future is taken out of its slot before drop. Polling, completion, cancellation and throwing drop cannot leave the frame available to a queued wake. A drop exception takes precedence over a poll exception, matching the optimized host driver. Owner tasks run both polling and drop under the captured owner and propagate through the existing DOM error policy, which preserves Error instances and normalizes other thrown values. Plain tasks propagate the original host value. This is driver cleanup, not general Rust unwinding or reconstruction of locals initialized inside an active poll. The panic policy is unchanged. `scripts/task-cleanup-test.sh` and `scripts/task-cleanup-browser.mjs` compare the compiled drivers, owner cancellation, cleanup order and exception identity; the control mode runs preserved original libraries against the same checks.

`scripts/active-poll-cleanup-test.sh` gates the separate active-call cleanup contract. Its safe Rust fixture covers first-poll locals, locals created after resumption, partial aggregate construction, an owned argument moved into an ordinary callee, and replacement whose right-hand side throws. Both Promise drivers run each case normally and with an injected host exception: 20 rows per build. The JIR lane builds under `js-profile=release-balanced`, so `js-cleanup=on`, in each of off/report/on, and all 60 rows must pass: every initialized local drops in order before the Promise settles, with the original value or the injected exception. off and report must emit byte-identical JavaScript, and each source map must check. The baseline lane has no semantic cleanup lowering, so in every mode the gate requires its compilation to fail with the `requires semantic cleanup lowering` refusal (see `js-cleanup=on` above) and leave no program (the source map may be written before the link refuses). The script retains every result and exits nonzero on any failing row, any baseline program, or any baseline failure other than that refusal. Passing driver cleanup does not satisfy this gate: a suspended future's drop shim cannot recover a local that exists only inside an active Rust invocation.

The owner is captured at `spawn`, before the first poll, and every resume runs inside `_$runWithOwner`. Owner cleanup cancels the task, drops its initialized future state under that same owner, and prevents later wakes from polling it. If cleanup occurs during an active poll, dropping the pinned frame waits until the poll returns. Already-issued native Promise or fetch work continues independently. Completion and repeated cleanup drop the future at most once.

The `rcgjs.host.spawnowner.v2` marker admits this cleanup policy in both compiler lanes. Its optimized driver receives `onCleanup` as a seventh argument. The older `rcgjs.host.spawnowner` marker retains its six-argument driver contract, so linking an older object does not silently change its lifetime policy. `rcgjs.tc.cleanup` registers a closure with the current owner's `onCleanup`; without an owner, it registers nothing.

An island's setup must stay synchronous whatever this does (CONTRACT-DOM 14.6), so the executor is
only ever reached from an event handler or an effect. That is a constraint the emitter does not
check and the crate documents; a continuation resuming after the hydration bracket builds fresh DOM
instead of claiming the server's, and reports nothing in a production build.

## Templates (stage 3)

A `view!` compiled for the client reaches the backend as calls to the marker functions in
`view-abi`, and the backend replaces them with dom-expressions emission. This is the seam where the
compiler meets the framework: everything above this section is about lowering Rust, and this is the
one place the backend knows what a framework is.

The reference the emitted calls are held to is `contract/CONTRACT-DOM.md` and the corpus under
`contract/fixtures/corpus/`. Both are read only from the backend's side.

### The markers

Each marker is an ordinary Rust function with an unreachable body, recognized by the
`link_section` it carries. Recognition happens in `abi.rs` **before** `is_foreign_item` and before
the callee resolves, because nothing else about the call is unusual: it would compile and it would
abort at run time.

| section | lowering |
|---|---|
| `rcgjs.tc.template` | interned `_$template` cloner, the walk, and the record its holes are filled through |
| `rcgjs.tc.hole` | one hole, written once |
| `rcgjs.tc.handler` | one hole, whose value is an event handler |
| `rcgjs.tc.effect` | one hole, driven by a closure |
| `rcgjs.tc.cond` | `_$memo` over a conditional's test, and an accessor that dispatches on it |
| `rcgjs.tc.list` | `[]`, the list of rows a `for` renders into |
| `rcgjs.tc.push` | `list.push(row)` |
| `rcgjs.tc.signal` | `createSignal(init)` |
| `rcgjs.tc.sget` | `pair[0]()` |
| `rcgjs.tc.sset` | `pair[1](value)` |
| `rcgjs.tc.content` | the value, or `null` for the branch that renders nothing |
| `rcgjs.tc.component` | `_$createComponent(thunk)`, inserted where a dynamic child would be |
| `rcgjs.tc.text` | the string a value renders as: a conversion, or its own `Display` |
| `rcgjs.tc.pushkeyed` | `list.push(__rt.keyed_row(site, key, row))` |
| `rcgjs.tc.etv` | `event.target.value || ""`, the text of the element an event came from |
| `rcgjs.tc.epd` | `event.preventDefault()` |
| `rcgjs.tc.cleanup` | `_$onCleanup(() => body(env))` |
| `rcgjs.tc.micro` | `__rt.microtask(() => body(env))` |
| `rcgjs.tc.settled` | `__rt.settled(value, (x, ok) => body(env, x, ok))` |
| `rcgjs.tc.owner` | `_$getOwner()` |
| `rcgjs.tc.withowner` | `_$runWithOwner(owner, () => body(env))` |
| `rcgjs.tc.hostval` | the identity: a host value read as the `V` its caller claims |

The backend reads `abi: u32` first and refuses a payload announcing anything but version 6, by
name. A payload it cannot read at all is a zombie at the call site naming the field that failed,
never a panic in the compiler.

**Adding a marker does not bump the version.** The version describes the shape of the payload and of
the value model, which is what an old backend cannot cope with silently. A marker section it has
never heard of is already loud: it reports "`rcgjs.tc.<name>` is not a `view-abi` marker this backend
knows", at the call site, naming the cause. So `etv` and `epd` were additive at version 4, and so
were the five executor markers; what took the ABI to version 5 is the two new handle TYPES,
`JsValue` and `Owner`, which are part of the value model rather than of the marker set, and what
took it to version 6 is the two template construction flags below, which are new payload fields.

An ABI bump does not move the sysroot. `ABI_VERSION` is read in one place, at a marker call site,
and `core`, `alloc` and `compiler_builtins` contain none: no rlib in the sysroot carries a
`TemplateData`. What decides whether an existing rlib is still readable is the object file's own
`FORMAT_VERSION`, which versions the item table, and it is unchanged. `build_sysroot.sh` is more
conservative than that and rebuilds on any change to the backend dylib, which is a build script
decision rather than an ABI requirement.

The executor imports `getOwner`, `runWithOwner`, and `onCleanup` from the configured DOM module. The vendored runtime exports these from one reactive graph, together with `createSignal` and the other reactive additions. A custom DOM module must provide the same owner and cleanup contract.

The two event markers take the event **by reference**, unlike the signal markers, which take their
handle by value (see "The handles are transparent, and taken by value"). The reason the two differ:
a handler closure's parameter already is a `&Event` -- that is the only way an `Event` is ever
reached -- and a marker's arguments are read by the backend rather than interpreted by the value
model, so what arrives is the DOM event the runtime passed the closure. Taking it by value would ask
the value model to copy a value out of a reference it has no representation for. `Event` therefore
stays an opaque struct with no handle field.

`|| ""` rather than `?? ""` on the target value: a `&str` return promises a string, and an element
with no `value` (a `<div>` the event bubbled from) would otherwise hand Rust `undefined`, which
breaks the moment anything reads its length. The two operators cannot differ here, because they
disagree only on falsy values that are neither `null` nor `undefined` and `target.value` is always a
string, whose one falsy value is the empty string this substitutes anyway.

### A component hole

A component invocation is a hole of its own kind, and it is inserted exactly the way a dynamic
child is: no anchor when it is an element's sole child, before the `<!>` when it has siblings, and
between the `<!$>`/`<!/>` pair in a hydratable template. `HoleKind::inserts_a_node` is what the walk
planner and the target locator ask, rather than naming `Child` twice.

What the kind changes is that the value is wrapped in `_$createComponent`:

```js
_$insert(root, _$createComponent(() => card$fn({ title: name, count: 3 })), anchor);
```

Three things about that shape are load bearing.

- **`createComponent` is what nests the hydration keys.** It swaps `sharedConfig.context` for a
  child context, runs the call, and restores the parent's already-advanced counter
  (CONTRACT-DOM.md 9.5). Routing the call through it is the whole reason the client half needs no
  key bookkeeping of its own.
- **One argument, not two.** Upstream's `createComponent(Comp, props)` is
  `untrack(() => Comp(props || {}))`, so a one-argument call passes `undefined` and the thunk is
  invoked with `{}`, which it ignores. The props object is not lost: a Topcoat client component is a
  plain Rust fn taking one props struct, and a struct argument is a JS object keyed by field name,
  so the object is inside the thunk and the callee destructures it. That is the equivalence family
  07 is built on, and `contract/fixtures/corpus/deltas.json` predicted this exact shape.
- **A component reaches the backend through `component`, not `hole`,** for the reason given below
  for `handler`: `hole<V>` cannot call a `V`, so a component passed through it would arrive with its
  body, the component function, and everything the component calls all missing from the program.

### Text conversion

`text(v)` is the string a hole's value renders as. The backend sees the monomorphized `T`, so most
values need no formatting machinery: `str` is already a string, a reference is read through (which
is what makes `(*title)` unnecessary for a `&&str`), `bool`, the integers and the floats go through
`String(x)`, and a `char` goes through `String.fromCodePoint` because a `char` is a code point
NUMBER in the value model and `String(x)` would print the number.

Anything else is `T`'s own `Display`, and that is ordinary Rust rather than an emission: the call
goes to `view_abi::display_to_str::<T>`, which streams into the host string builder (`sb_*`, see
"Allocation") and returns the finished string. The marker's body names that function so the
collector walks into it, and the backend finds it beside the marker rather than by spelling a path.
`alloc::String` takes this path too: it is a `Vec` of bytes in the value model, not a JS string.

### Keyed rows

`push_keyed(&list, key, row)` appends a row matched to the row of the same key from the previous
render:

```js
__tc_rows.push(__rt.keyed_row("<crate>#<n>", <key string>, <row>));
```

The key is converted exactly as a `text` value is, so a key is compared by the text it renders as.
`__rt.keyed_row` keeps one `Map` per loop; the site is `<crate name>#<n>` rather than a bare number
because the counter is per codegen unit, and two loops compiled in different crates would otherwise
share one cache, which is a wrong-rows bug rather than a slow one.

This is a pragmatic reconcile and not the runtime's `_$mapArray`. A view's `for` is an eager Rust
loop, so every row is BUILT on every render and the row of a cached key is then discarded in favour
of the node that key already had. What it buys is node identity: reordering a list moves the
existing nodes rather than replacing them, so whatever DOM state they carry survives. The cache is
not pruned, so a key that stops appearing keeps its node alive. Keyed tracking that skips building
needs the collection handed over as a JavaScript value plus a row function the runtime calls, and a
Rust iterator is neither.

Two marker shapes exist for reasons that are not about what they emit:

- **`handler` rather than `hole`** for an event handler, because a marker's body is what makes the
  closure it is handed reachable: the monomorphization collector walks into a closure only through
  a call, and `hole<V>` cannot call a `V`. `handler`'s bound is `Fn(&Event) -> R`, and its body
  builds the event and calls the closure. The value is discarded, as a DOM listener's is.
- **`cond` rather than one closure over the whole conditional**, because a test buried in a closure
  that also builds the branches cannot be hoisted; see "Memo hoisting" below.

`list`/`push` is how a `for` is lowered, and the design is the one the corpus's own family 13 notes
call the structural match: the loop stays Rust. Its iterator is a Rust iterator, so nothing the
backend could synthesize would iterate it; instead the loop runs where it was written, appends each
row to the list, and the list of DOM nodes is one value `_$insert` takes. That renders a list rather
than tracking one, and `view_abi::list` says so: a keyed, per-row `_$mapArray` needs the collection
handed over as a JavaScript value plus a row function the runtime calls, and neither is expressible
while the iterator is a Rust one.

### Arguments the backend reads instead of lowering

Some marker arguments never appear in the emitted JavaScript: a `template` payload, which is
decoded at codegen time; a hole index, which is a compile time constant; the `&root` of a hole and
the `&list` of a `push`, which name something the backend is already holding; and a signal ordinal,
which is dropped. Nothing is left to consume the MIR that computed them, so `abi.rs`'s
`consumed_locals` drops it: the local carrying such an argument, and each local behind it, has its
assignment skipped. Such a borrow also forces no box; see "References and raw pointers".

The rule is narrow on purpose. A local qualifies only when it is assigned once, read once and
never borrowed, which means its one reader was the argument that was just consumed and its
assignment has nothing left to feed. The walk follows the moves and borrows an expansion writes and
stops at anything else, so a body this pass does not fully understand keeps every store it had.

This is what makes the payload `static` disappear. Nothing keys on it and nothing could: it carries
no attribute, and a structural match on its type would recognize a type rather than a use. What
removes it is the reachability pass in `link.rs`, which drops any binding no reachable item
mentions. The one thing that kept it mentioned was the dead store, and the dead store is gone.

### One decoder, two readers

`backend/src/constant/raw.rs` holds the byte level primitives: reading a scalar, decoding a
`Variants::Multiple` tag (direct and niche), and following the provenance and length behind a
`&str` or a `&[T]`. Two readers sit on it, and neither has a decoder of its own:

- `constant.rs` turns a constant into a JavaScript expression, which is what a `static` the
  program keeps needs;
- `constread.rs` turns the same bytes into a `ConstVal` the **backend** reads, which is what a
  `static` the backend consumes at codegen time needs.

A second, independent decoder is the failure mode this arrangement exists to prevent: it would
agree with the first on every case anyone tested and disagree on the one nobody did.

### The walk

A hole's `path` is a run of `c` (`firstChild`) and `n` (`nextSibling`). `Walk::plan` names **every
prefix of every needed walk** exactly once, which is both what shares work between two holes under
one element and what CONTRACT-DOM 3.2 describes: only nodes on the path to a dynamic position are
named at all.

A `Child` hole's walk reaches one of two things and the emitted call differs:

```js
_$insert(el, value)                  // the value is the element's sole child
_$insert(parent, value, anchor)      // the value goes before an anchor comment
```

The two are **indistinguishable from the walk alone**: `c` is both "the first child element" and
"the first child, which is an anchor", so the template's HTML is parsed and the node the walk
reaches is looked at. `template.rs`'s `Tree` is that parser: elements, text runs and the three
comment forms, over the closed grammar the emitter writes rather than over HTML in general. A walk
that reaches no node is a hole error naming the walk, never a guess.

The element an anchored child inserts into is the walk with its last descent dropped: an anchor is
a child position, so its walk is its element's walk plus one `c` and a run of `n`.

A hydratable template's walk moves over the **server's** nodes, and there the pair that brackets a
dynamic child has the rendered value between its two markers while the template says the markers are
siblings. Two things follow, and both are part of the walk rather than of the hole:

```js
$t1 = $t0.nextSibling;                     // the `<!$>`
$t2 = _$getNextMarker($t1.nextSibling);    // the `<!/>`, and the nodes between the two
$t3 = $t2[0].nextSibling;                  // whatever the template has after the pair
_$insert(parent, value, $t2[0], $t2[1]);
```

- **`_$getNextMarker` is handed the node after the `<!$>`**, never the marker itself. It scans
  forward for the matching `<!/>` counting nested pairs, so handed the opening marker it counts that
  one and runs off the end: the value's own nodes are not collected and the insert replaces what the
  server wrote instead of adopting it. One DOM mutation per dynamic text hole is the symptom.
- **A walk that continues past a pair re-bases on the `<!/>` the call returned.** Counting siblings
  from the `<!$>` lands inside the rendered value. So the closing marker is named by the call rather
  than by a `nextSibling`, and everything after it is walked from there.

Both are upstream's shape, node for node
(`contract/fixtures/upstream/__dom_hydratable_fixtures__/textInterpolation/output.js`); the one
spelling difference is indexing in place of `[_el$, _co$] =` destructuring, and the same two values
reach `_$insert` in the same order. Because the call is in the walk, a hole whose pair another walk
also needs shares one `_$getNextMarker`, exactly as two holes share a prefix.

`examples/dom-tests/18_hydratable` is the fixture, and its `after_two_holes` case is the one that
needs both rules: two dynamic children under one element with static text between and after them.

### Which sink a hole reaches, and what it is handed

`_$insert` takes a value **or** an accessor and subscribes to the accessor itself, so a reactive
child hole needs no effect around it and is handed `() => body(env)`. Every other reactive sink is
written inside an `_$effect` and is handed the call, `body(env)`, because what it writes is the
value the closure returned rather than the closure. An event sink is handed a function whether or
not it re-runs, because the sink is what calls it.

`_$classList` and `_$style` diff against what they returned last time, so a reactive one takes the
effect's previous value as a third argument: `_$effect(_$p => _$style(node, value, _$p))`. That is
upstream's shape for a lone diffing sink (CONTRACT-DOM 6.2).

### Template construction flags, and the SVG wrapper

`_$template` takes three booleans after the HTML, and the payload carries the two of them a Topcoat
program can reach: `is_import_node` and `is_svg`. `isMathML` is always false, because the emitter
knows its own namespaces and has no MathML lowering. The call is emitted with all three arguments
when either flag is set and with the HTML alone otherwise, which is what the reference compiler
emits (CONTRACT-DOM 2.4).

`view-dom`'s `Template::finish` decides both:

- **`is_import_node`** when any element in the template has a dashed tag name. The runtime then
  instantiates with `document.importNode` instead of `cloneNode`, so a custom element upgrades on
  insertion. The decision is per template, not per element: one dashed descendant flags the whole
  template.
- **`is_svg`** when the template's ROOT is an SVG-only element. `<svg>` itself is not one: it is an
  ordinary HTML element to the fragment parser, so a template rooted at one parses in the right
  namespace already and takes no flag. The element list is generated by `view-dom/build.rs` from
  `contract/fixtures/properties.json`'s `SVGElements`, the same extracted table the property and
  delegated-event classifiers read, so it is upstream's list rather than a copy of it.

**`is_svg` and the `<svg>` wrapper are one decision and must never be made by halves.** Under the
flag the runtime returns `t.content.firstChild.firstChild`, one level deeper than usual
(CONTRACT-DOM 2.2), because the template string is wrapped in a literal `<svg>` first. So
`Template::finish` writes the wrapper into the HTML, and `Template::tree` roots the walk at the
wrapper's first child to match. Hole paths are untouched: they stay rooted at the element that was
written, which is the node the runtime hands back.

Emitting the flag without the wrapper, or the wrapper without the flag, **silently yields the wrong
root node**. Nothing reports it. A walk read one level too high answers `kind_at` with the wrong
node kind, which turns an anchored child insert into a sole-child one and back, so the emitted call
is well formed and inserts into the anchor comment at run time.
`examples/dom-tests/24_svg_wrapper_walk.rs` is the pin: its `root_group` case is a shape whose two
readings differ, and its golden is what tells them apart.

The wrapper is closed, unlike the trailing tags a template may leave open, because the extra unwrap
counts on it. That makes it the one template string the reference does not minify all the way
either, which the L2 harness accounts for with a family-09 rule.

### Interning and declaration order

A cloner is a module level `const` interned on **the HTML text and the two construction flags**,
never a def path: two crates that build the same template must name one cloner, and a def path is
spelled relative to whoever printed it (see `naming.rs`). That is what makes the runtime's
per-cloner memoisation per template rather than per render (CONTRACT-DOM 2.1).

A template with neither flag keys on `tmpl\x01<html>` and a flagged one on
`tmpl\x01<flags>\x01<html>`, which cannot collide with it because a template's HTML always opens
with `<`. The flags belong in the key because they are part of the cloner rather than of the text:
two templates with one HTML text and different flags are two different construction routines, and
interning them together would give whichever was emitted second the first one's `importNode` or
unwrap depth.

Interning on the HTML means a cloner's *name* is a hash, and a name is what `link.rs` used to order
module level bindings by. The reference compiler declares one `_$template` per distinct string in
first-use order, and a trace numbers templates in declaration order, so the order is observable: a
permuted set of cloners shows up as a difference on nearly every record.

So a cloner carries a `SourceOrder`, the position of the `view!` that built it, and `link.rs` places
the bindings that carry one before the rest and in that order. It is the position of the macro call
rather than of the `static` the expansion generated, which is what "first use" means in the
reference output. Two more rules make that order the source's rather than an accident:

- **The earliest use wins.** A template several views build is interned by the first *codegenned*
  use, and codegen order is a symbol-hash order with nothing to do with the source. Every use offers
  its position (`CguCx::order_at_most`) and the smallest one stays. Only uses in one codegen unit can
  offer one, which with one unit per crate is every use in the crate.
- **Two templates of one view tie-break on the index the emitter numbered the payload with.** The
  span cannot answer this: a proc macro's output all carries the macro call's span, so every payload
  of one view has the same one. `__TC_TEMPLATE_2` is the third template the view mentions, which is
  where the reference declares it.

Adding the field and then the tie-breaker bumped the object file's `FORMAT_VERSION` to 3.

Runtime names are `ItemKind::Import` items from the `js-dom-module` specifier (default
`topcoat-dom`), one item per name, bound as `_$template`, `_$insert`, `_$effect` and so on: the
`_$` convention upstream's output uses, so a diff against the reference is about code rather than
spelling. They deduplicate, order and survive dead code elimination through the machinery
"Modules" already describes.

Because those are imports, **a program using templates must be compiled with
`-Cllvm-args=js-modules=esm`**. In script output an `import` is a syntax error in the file rather
than a wrong value, so the instantiation is refused with a zombie saying so instead.

### `delegateEvents`

Emitted with the instantiation, inside the compiled function, so the program keeps having no module
level side effects. The reference emits one call at the end of the module carrying the union over
every template; ours is one call per instantiated template that needs one, placed before its holes
are filled. Both register the same listeners. The event list is deduplicated and sorted by the
backend rather than trusted from the payload, because a duplicate would register one listener
twice.

### Reaching a handler's body

The monomorphization collector walks into a closure only through a call, so a closure handed to a
marker is collected exactly when the marker's body calls it. Every marker that takes one does:
`cond` calls all three of its closures, `effect` calls its own, and `handler` builds an `Event` and
calls the handler with it. `Event` is constructible only inside `view-abi`, which is what lets that
body exist at all.

Function items passed to these markers or to `topcoat-js` callbacks use their signature metadata
for argument counts. A non-generic function in another crate can have compiled code in its rlib
without exporting MIR to the caller. That absence does not prevent calling the function. Closure
environments still use their MIR argument layout. `scripts/function-item-callbacks-test.sh` checks
external zero-, one-, and two-argument functions, exception identity, generic functions and closures
in both lanes and all three optimizer modes.

So there is no walk of the backend's own here, and there was one: while a handler reached the
backend through the fully generic `hole<V>`, which cannot call a `V`, the backend re-walked the
body the way the collector would. That is gone with the marker that made it necessary. A handler
calling anything at all used to name a function no crate defined -- 17_signal's golden carried a
throwing stub for `Sig::set` -- and the marker is what fixed it, not a second collector.

A handler's parameter also has a type now, which is what lets one be written the way the corpus
writes it. `view-dom` normalizes three forms into `handler`'s `Fn(&Event) -> R`: a closure written
in place keeps its body and has a bare parameter annotated; a **name** is the function to call and
is passed through; any **other expression** is the body to run, which is what Topcoat's
`@click=$(count.set(1))` means, and becomes the body of a closure over the event.

### A thunk built inside a loop snapshots its environment

A closure reaches a marker as a **thunk**: `($a0) => body(env, $a0)` for a handler, `() => body(env)`
for an accessor, and the bare call `body(env)` where the marker writes inside an effect of its own.
`env` is the expression the closure's environment lowered to, which for a `move` closure over the
loop variable is the *binding* of a local.

Every local is declared once, in the function prelude ("Emitted JS shape"), so a `for` over the rows
of a grid assigns a fresh environment array to the same binding on every turn. An arrow closing over
that binding would read whatever the last turn left there, and every row's handler would act on the
last row. So a thunk whose creation site lies on a control flow cycle takes the environment as the
parameter of an immediately invoked wrapper:

```js
$t2.$$click = (($c) => ($a0) => per_row$closure_1($c, $a0))([picked, row]);
```

`$c` is in the `$` namespace the backend reserves for its own names, so it can never shadow a local.
Outside a cycle the arrow is emitted bare: nothing reassigns the binding, and the wrapper would only
be noise. The **call** form is never wrapped, because it runs before the loop moves on.

"Inside a cycle" is measured on the same graph the structurizer is handed -- the non-cleanup blocks
and the edges between two of them -- so it means the same thing here as it does in the emitted
JavaScript. It is not gated on `js-scoped-lets`, and it holds in the dispatch fallback too, where
every block shares one `switch` scope and the binding is even more obviously shared.

### The handles are transparent, and taken by value

`Node` and `Sig<T>` are both `#[repr(transparent)]` one-word handles, and the JS value the backend
keeps in one is a DOM node, a list of them, or the runtime's `[get, set]` pair. Transparency is what
makes that possible: in the value model an ordinary struct is an object keyed by field name, and a
value copied out of one is REBUILT field by field, so a handle copied by value would come back as
`{ handle: undefined }` and reading it would call `undefined`. Transparency makes both the field read
and the rebuild the identity.

`sig_get`/`sig_set` therefore take `Sig<T>` **by value**, and `Sig::get`/`Sig::set` take `self` by
value, which is why `Sig<T>` is `Copy`. The alternative fails in the other direction: a `&Sig<T>` is
a reference to a one-word primitive, so the value model boxes the local as soon as anything takes its
address and the marker is handed the box rather than the pair. Taking the handle by value removes the
address-taking, and a copy of a handle correctly names the same signal.

A non-`move` closure still captures a `Copy` handle by reference, so a local a closure reads does
box, and the read through it is spelled `env[0].buf[env[0].off]` -- the pair is read out of the box
before the marker sees it, which is correct but costs an indirection. Emitting a view's reactive-hole
and handler closures as `move` closures would remove it.

### Effect groups

Holes of one `effect_group` collapse into a single `_$effect` carrying a `_p$` previous-value
record, per CONTRACT-DOM 6.2:

```js
_$effect(_p$ => {
  let _v$ = first(env), _v$2 = second(env);
  _v$ !== _p$.e && (node.disabled = _p$.e = _v$);
  _v$2 !== _p$.t && _$setAttribute(node, "title", _p$.t = _v$2);
  return _p$;
}, { e: undefined, t: undefined });
```

Every reactive hole still arrives as its own marker call, so the effect cannot be written until
every write it carries is known: a hole of a shared group is buffered, and the last one of the
group to arrive emits all of them. `view-dom` groups the reactive binds of one element and puts a
child hole or an event hole in no group, so a group's holes are consecutive and an expansion fills
them in order. A group that never completed is a group whose writes were buffered and never
emitted, which is a miscompilation rather than a missed optimization, so `codegen_body` records a
zombie naming it.

A group of one stays a plain effect. A record with one key buys nothing, and it is also what
upstream emits.

Record keys come from upstream's frequency ordered alphabet (CONTRACT-DOM 6.3) and the
value locals are `_v$`, `_v$2`, and so on. Both are private to one effect, so any unique scheme
would be correct; matching upstream is what makes the emitted code diffable against the reference.
Upstream numbers `_v$` across a whole function and this numbers it per effect, which is the one
spelling difference left in the shape.

A note for CONTRACT-DOM's owners, unchanged from when it was found and repeated because nothing has
acted on it: **6.3's heading says "base 53" and the alphabet it quotes is 54 characters long.**
Upstream divides by `chars.length`, so the real base is 54; `record_key` follows the string, and its
unit tests pin `record_key(53) == "$"` and `record_key(54) == "te"`. `contract/` is read only from
here, so this is reported rather than corrected.

### Memo hoisting

A conditional's test is hoisted into `_$memo`, so an unchanged test does not rebuild the branch
(CONTRACT-DOM 6.4):

```js
$t0 = _$memo(() => test(env));
_$insert(root, () => $t0() ? then(env) : els(env));
```

which is the reference compiler's conditional
(`__dom_fixtures__/conditionalExpressions`) with its immediately invoked wrapper flattened: that
wrapper exists to make the memo's declaration an expression, and a statement position needs none.
The reference also writes `!!test`; here the test is a `Fn() -> bool`, so its value is already a
JavaScript boolean and the coercion has nothing to do.

What makes it possible is that the test arrives as a closure of its own, through `view_abi::cond`.
Handed one closure over the whole conditional the backend has no test to hoist, only a function that
computes a branch, and producing the shape above would mean splitting one MIR body into three.

Both spellings of a conditional go through `cond`: a markup-bodied `if`/`else if`, and a Rust `if`
inside `$(..)` whose branches are values rather than markup. An `else if` chain nests, the outer
`else` closure's body being another `cond`, which is how the reference nests its own memos too.

Two shapes are deliberately left as one closure, and `view_abi::cond` says why: a `match`, and an
`if let`. Their tests are patterns rather than booleans and an arm's body reads what the pattern
bound, so handing the test over separately would mean matching once to choose the arm and again
inside it to bind, which is a different program from the one that was written. Family 12's remaining
`memo` delta is exactly this.

### What version 3 of the ABI changed

Three defects the previous milestone found were in `view-abi`, and version 3 is those fixes. All
three were fixed in the ABI rather than worked around in the backend, and the backend workaround
each one had is gone.

**1. `Node` is a `repr(transparent)` one-word handle.** It used to be `Node(())`, zero sized, so a
template root passed as a value was folded to a constant and reached the backend as `undefined`:
every branch of a markup-bodied `if` or `match` built its template and then contributed nothing, and
an island entry point returned nothing. Both halves of the declaration are load bearing. Non-zero
sized, so the value survives; **transparent**, so the handle *is* its field, because an ordinary
one-field struct is an object keyed by field name here and a value copied out of one is rebuilt field
by field -- which turned a root passed through a `Node` into `{ handle: undefined }`.

**2. `handler` exists,** so a handler's body is reachable and its parameter has a type. See
"Reaching a handler's body".

**3. `each` is `list`/`push`,** so a `for` lowers. See "The markers".

### The suite

**In this repository the suite is framework-specific, and skipped.** Every fixture in
`examples/dom-tests/`, and every one in `examples/keyed-tests/`, is written with `view_dom_macro`, a
Topcoat framework crate this repository deliberately does not carry (docs/V2-RESTORATION.md).
Without a `view-dom-macro/` checkout, `scripts/dom-test.sh` and `scripts/dom-matrix-test.sh` print
`SKIP: framework-specific suite (needs view_dom_macro; see CONTRACT.md)` and exit 3 before building
anything. `scripts/lane-shadow-test.sh` leaves `dom-tests` out of its default suites; named, it is
one `SKIP framework` row, which its `N/M agree, K skipped` summary never counts as an agreement.
`scripts/jir-coverage.sh` records the suite as `skipped-needs-framework`. `scripts/matrix.sh`'s
`checks` row reports `for-handler-test`, `keyed-compiled-test`, `island-sourcemap-test` and
`artifact-manifest-test`, which need `view-dom-macro` or `demo-app`, as `SKIP framework` lines,
neither passed nor failed. The committed `NN.js.expected` and `NN.trace.expected` are frozen
September 7 data that no gate here re-emits, so `contract/harness/check-l2-status.mjs` measures
those frozen traces and not this compiler's output. The rest of this section describes the suite
as it runs with a framework checkout.

**The generic subset.** Two compiler behaviours were pinned only by DOM fixtures. Each now has a
macro-free fixture, run in `scripts/matrix.sh`'s `checks` row:

- `examples/host_loop_callbacks.rs` (`scripts/host-loop-callbacks-test.sh`) is what
  `23_for_row_handlers` pinned. Closures created directly in `for` loop bodies, handed to the host
  and called after the loop, must answer their own turn's captures: by value, through a reference
  to a `Copy` value, from a nested loop, together with an outer counter they mutate, and a running
  local the loop reassigns (see "A thunk built inside a loop snapshots its environment"). The
  release build (`js-profile=release-balanced`, so `js-cleanup=on`, on the JIR lane) keeps
  `callback0` out of line, so each thunk is created in a call frame of its own. The inlined build
  (`JSC_RUST_MIR_INLINE=on`, both lanes) puts the marker on the loop itself, and there the check
  requires the immediately invoked `(($c) => ($a0) => body($c, $a0))(env)` wrapper at every site.
  On the parsed program, both builds require that no function created on a loop escapes over a
  binding of the enclosing function. Optimizer off, report and on; off and report byte identical.
- `examples/host_event_access.rs` (`scripts/host-event-access-test.sh`) is what
  `21_event_accessors` pinned, together with the listener around it. A `#[js_extern]`
  `addEventListener` receives a closure that reads `event.target.value` (the empty string when there
  is no string value) and calls `preventDefault()` once, on the event it was handed. A second
  listener holds a `Drop` guard across `preventDefault`, which the check makes throw an Error, an
  object, a symbol, `undefined` and `null`. The guard must drop exactly once, nothing after the call
  may run, and the caller must receive the thrown value itself. JIR lane under
  `js-profile=release-balanced`, optimizer off, report and on; off and report byte identical.

`examples/dom-tests/` holds one fixture per corpus family it mirrors, written against
`dom_view_client_only!` because the reference traces were recorded from the non-hydratable preset.
Each carries `NN.js.expected` and `NN.trace.expected`, and `NN.family` names the corpus family its
trace is compared against where one means the same thing.

Two fixtures mirror no family. `16_topcoat_spread` pins an emission shape the corpus writes with a
type the spike does not build. `18_hydratable` is the only one compiled with `dom_view!`, the mode an
island's client half uses: it pins the marker-pair walk described above, which is invisible in a
client-only template, and its `twelve_holes` case pins that nothing emitted depends on how hydration
keys are *spelled*. The client asks the runtime for them (`_$getNextElement`, `_$getNextMarker`),
which counts with `sharedConfig` and looks the node up in the registry the server's `data-hk`
attributes filled, so a change to the server's encoding cannot move a walk.

Compiling one needs two things the `core` suite never does: `view-abi` built by this backend into
an rlib, and `view-dom-macro` built by plain cargo **for the host**, because a proc macro runs in
the compiler's own process whatever `--target` says. `compile_core.sh` grew `--extern NAME=PATH`
and `--crate-type` for both. There is no pre-expansion step: rustc loads the host macro dylib and
compiles the fixture in one pass, and the output is byte for byte what expanding first produced.

L2 (trace parity, corpus README) is reported per family rather than allowed to fail the suite: a
difference there is a finding about two emitters and not necessarily a regression in this one.
`examples/dom-tests/deltas.json` holds the accepted ones and, deliberately, holds only differences
that are total and mechanical: a template spelling convention, a field the reference's own
execution fills with a healed free binding. A difference in what is **emitted** is never a rule.
Those are listed in that file under `$whatIsDeliberatelyNotHere`, per family, so that every line a
comparison still reports has a written cause: anchor placement (view-dom's, documented in
`view-dom/src/lib.rs`), Topcoat's write-once `(expr)` against JSX's always-reactive attribute,
`delegateEvents` placement, a `match`'s missing memo, a reference program written with a component
Topcoat has no form for (`<Show>`, `<For>`), and the cases a family has that no fixture here can
write yet. A delta that gets fixed moves to `$measuredAndRetired` with the measurement that pins the
fix, rather than being deleted.

## Declared JavaScript interfaces (`#[js_extern]`, stage 4)

A program declares an interface to hand-written JavaScript with the `js_extern_macro::js_extern`
attribute over a foreign block, and the backend emits the JavaScript operation each declaration
names.

```rust
#[js_extern(module = "chart.js")]
unsafe extern "C" {
    #[js(new = "Chart")]
    fn chart_new(target: JsValue, config: JsValue) -> JsValue;

    #[js(get = "Chart.version")]
    fn chart_version() -> JsValue;
}

#[js_extern]
unsafe extern "C" {
    #[js(method = "update")]
    fn update(chart: JsValue, data: JsValue);

    #[js(index = "data.datasets")]
    fn dataset_at(chart: JsValue, index: f64) -> JsValue;

    #[js(method = "getPoint", nullable)]
    fn get_point(chart: JsValue, index: f64) -> Option<JsValue>;
}
```

### Nothing foreign survives the expansion, and it must not

Each declaration becomes a **marker function**: an ordinary Rust `fn` with a body, carrying the
descriptor in a `link_section`, exactly as `view-abi`'s markers do. The block is written as a
foreign one because that is the shape the signatures suit, and rustc rejects the attribute on what
it looks like: *"the `link_section` attribute cannot be used on foreign functions ... this was
previously accepted by the compiler but is being phased out; it will become a hard error"*. The
help text names the shape that works, and that is the shape the macro writes.

The call is intercepted in `abi.rs`'s `codegen_call`, in the arm beside the `view-abi` markers and
ahead of `is_foreign_item`, so the marker body is never reached. Every body is then unreachable
from anything and dead code elimination removes it, along with the `core::fmt` chain its
`unreachable!` would otherwise pull in.

### The descriptor

```text
rcgjs.ext.<version>.<shape><flags>.<slots>.<module-len>,<name-len>.<module><name>
```

for example `rcgjs.ext.3.n-.-.8,5.chart.jsChart`. `<flags>` is a run of letters or `-` for none:
`n` is a nullable return and `g` is a path rooted at the global scope. `<slots>` is the per-slot
host contract, `-` where the declaration annotated none: a `,` separated run of `<key><letters>`
entries in slot order, naming the carriers the host accepts at that slot and how it observes the
value. Version 2, the same grammar without `<slots>`, is still decoded and annotates nothing. The fields are length prefixed rather than
delimited because a module specifier holds `.`, `/`, `-`, `@` and `:` and a JavaScript name holds
`$` and `_`: no character is safely excluded from both, a delimiter would need escaping, and an
escaping bug is silent. Decoding is total, and the version is the first field read, so a descriptor
this backend does not understand is a zombie naming the descriptor rather than a guess.

The format lives in one file, `js-extern-macro/src/descriptor.rs`, which the macro compiles and the
backend includes with `#[path]`. A proc-macro crate can export nothing but proc macros, so sharing
it any other way would have meant a third crate carrying one file, and two copies of a format is how
a format drifts.

### The shapes, and where each is rooted

**Three roots exist and every declaration has exactly one:** the call's first argument, a module's
imported binding, or the global scope. A module specifier names the second, the `g` flag
(`#[js(global)]`) names the third, and the absence of both leaves a member shape on its argument.
A call and a construction have no receiver, so with no module they are already global and need no
flag. An index is the one shape a module specifier does not move: a collection is reached THROUGH
something, so `chart.data.datasets[k]` inside a block that names a package is the ordinary
declaration. The name is a dotted path walked from wherever it is rooted, so one declaration covers
a walk.

| `#[js(..)]` | rooted at | emitted |
|---|---|---|
| `call = "Chart.register"`, module given | the imported `Chart` | `Chart.register(a, ..)` |
| `call = "f"`, no module | a global | `f(a, ..)` |
| `new = "Chart"`, module given | the imported `Chart` | `new Chart(a, ..)` |
| `new = "EventSource"`, no module | a global | `new EventSource(a, ..)` |
| `method = "update"` | argument 0 | `a0.update(a1, ..)` |
| `get = "data.datasets"` | argument 0 | `a0.data.datasets` |
| `get = "Chart.version"`, module given | the imported `Chart` | `Chart.version` |
| `global, get = "dash.status"` | a global | `dash.status` |
| `set = "title"` | argument 0 | `a0.title = a1` |
| `index = "data.datasets"` | argument 0 | `a0.data.datasets[a1]` |
| `index_set` | argument 0 | `a0[a1] = a2` |

The flag exists because a member shape has an argument to fall back on and a call does not. Without
it, `#[js(get = "dash.status")]` with no module reads a property of ARGUMENT ZERO -- silently, since
a declaration taking no arguments gets `undefined.status` and one taking an unrelated argument reads
a property of that. `global` also CLEARS a block's module rather than sitting beside it, which is
what lets a browser global be declared inside a block that names a package; `#[js(global, module =
"x")]` is a spanned error naming both roots.

A call and a method are the same emission: the path's last step is called as a member, so a static
keeps its receiver. A walk is emitted step by step rather than collapsed, because each step is a
property read the interface may be doing deliberately.

A declaration that names a module binds it with an `ItemKind::Import`, whose local is keyed on the
module and the name together: two modules exporting `Chart` bind two locals, and two crates
importing the same `Chart` bind one, which the linker then collapses. Only the path's FIRST segment
is imported, because `import { Chart.register }` is not a thing anyone can write.

### Arguments pass through

The value model already spells a number as a number, a `&str` as a string and a `#[repr(C)]` struct
as an object keyed by field name, so a declared interface receives the arguments it was declared
with and nothing is marshalled. What a shape adds is position: the receiver, the key an index reads
and the value a setter writes are taken from the front of the argument list, and everything after
them is passed on in order. **Nothing is padded**: a declaration with fewer parameters produces a
call with fewer arguments, which a library that branches on `arguments.length` can see.

An opaque JavaScript value is declared on the Rust side as a `#[repr(transparent)]` struct holding
one machine word, for the two reasons `view_abi::Node` gives: a zero sized value is folded away by
MIR, and an ordinary one field struct is rebuilt field by field, so a JavaScript object passed
through one would come back as `{ handle: undefined }`.

### Byte buffers cross by copy

Passing through holds for a byte slice too, which is why it is the wrong way to hand bytes to the
host: a `&[u8]` is the `{ buf, off, len }` record over a JavaScript array of numbers (or over a
typed backing store), not a `Uint8Array`, and an interface declared to take one receives the
record. `mires_js::JsUint8Array` is the binding for bytes, and it crosses in bulk:

| Rust | markers | runtime |
|---|---|---|
| `JsUint8Array::from_slice(&[u8])`, `From<&[u8]>` | `rcgjs.host.u8arrayfrombytes` | `__rt.u8array_from_bytes(s)` |
| `JsUint8Array::len(&self)` | `rcgjs.host.u8arraylength` | `__rt.u8array_length(view)` |
| `JsUint8Array::copy_to_slice(&self, &mut [u8])` | `u8arraylength`, then `rcgjs.host.u8arrayintobytes` | `__rt.u8array_into_bytes(src, dst)` |
| `JsUint8Array::to_vec(&self)`, `From<JsUint8Array> for Vec<u8>` | the same two, into a fresh zeroed `Vec` | |

The markers are `rcgjs_object::host::HostOperation`s, so the two lanes lower them from one table.

- **Bulk, whatever the length.** The export is one host operation; the import is two, the length
  read that sizes the destination and one copy. The loops are in the runtime helpers, so a 61 KB
  buffer costs one or two calls rather than one boundary call per byte.
- **Copy, never alias.** The export returns a fresh `Uint8Array` over its own `ArrayBuffer`; the
  import writes Rust storage with the element stores Rust itself would make. A write on either side
  after a crossing is never visible on the other.
- **Exact values and length.** The export reads `[off, off + len)` and nothing past it, so a `Vec`'s
  capacity never crosses. A store into a `Uint8Array` wraps, so `i8` storage crosses as its byte,
  as `__rt.bytes_str` reads it; typed backing storage is copied with one `TypedArray.prototype.set`.
  A zero length slice reads nothing, and its buffer may be absent or the frozen dangling array.
- **Lengths come from internal slots.** `len`, and so the import's sizing, read the
  `%TypedArray%.prototype` length and tag accessors the runtime captured when it loaded, never the
  view's `length` property: an own `length` a host defines on the instance changes nothing.
  `copy_to_slice` panics when the lengths differ, before it writes, as `<[u8]>::copy_from_slice`
  does, and `u8array_into_bytes` checks them again and aborts on a mismatch, so the helper is
  fail-closed on its own.
- **Detached views have length zero.** A view whose buffer was transferred after the conversion
  (conversion itself refuses a detached one) answers `len() == 0`: `to_vec` returns an empty `Vec`
  and `copy_to_slice` panics unless its destination is empty.
- **What may throw.** The export raises nothing but a registered compiler abort, which a cleanup
  handler rethrows before it runs any destructor; `try_host`, which catches everything, still
  answers one as `Err`. It checks its record before reading through it (integer length and offset,
  a plain array or typed array buffer that holds the range) and turns anything else that goes wrong
  into the same abort: a host object retyped as a byte slice by a declaration that lies, storage
  that is not bytes viewed as bytes through `unsafe`, an allocation the engine refuses. Such a
  misuse can still run host code first (a getter on the retyped object, a `valueOf` on a viewed
  element), but the operation itself cannot raise an ordinary exception, so
  `HostOperation::may_throw` answers `false` and a cleanup pass may remove the unwind edge around
  it. That is a statement about this operation only: the compiled path to it first reborrows the
  slice (`__rt.byte_slice_ref`, which reads the record's `buf`), and on a retyped host object that
  read can raise an ordinary exception before the operation runs — the same trust in Rust-valid
  operands that every `&[u8]` reborrow, load and length read already rests on. The length read runs no host code at
  all; a `JsUint8Array` that does not hold a `Uint8Array` (a declaration that mistyped a host value)
  is refused with the `TypeError` a mismatched `JsValue::read` raises, so destructors run as for any
  host exception. It, and the copy, which writes through a destination record it does not
  validate, answer `true`.

`examples/extern-tests/03_bytes_bulk.rs`, driven by `scripts/extern-check.mjs`, checks each of
these: empty, one and 65,536 bytes, 0 and 255, a slice at a non-zero offset, a `Vec` after `push`
and `truncate`, `i8` storage, round trips both ways, a length mismatch, copy semantics in both
directions, a shadowed `length`, a detached view, an abort for the malformed records and non-byte
views it lists, and a `TypeError` with no host code run for a mistyped view, plus the runtime
helpers directly on typed, dangling and hostile values. One silent case remains and predates this
operation: non-byte storage viewed as bytes through `unsafe` copies element-wise when the byte
length fits the element count and the elements are numbers (`u32` words `[0x01020304, 5]` viewed
as two bytes export `[4, 5]`).

### `#[js(nullable)]`

A declaration whose return type is `Option<T>` may say that the interface answers `null` for an
absent value. The emission holds the result in a temporary and tests it:

```js
$t0 = chart.getPoint(99);
_3 = $t0 == null ? { TAG: "None" } : { TAG: "Some", _0: $t0 };
```

The temporary is not an optimization: both arms read the value, and an interface may be called for
its effects as well as its answer, so a repeated expression would perform the operation twice. The
test is `==` rather than `===`, so a missing property (`undefined`) is `None` as well as an explicit
`null`. The two `Option` values are built by `value::enum_value` from the destination's own type, so
this lowering has no opinion about the `Option` encoding and cannot drift from it. A destination
that is not an `Option` is a zombie.

This is opt-in and per declaration, which is the whole difference from a sentinel encoding: nothing
about `Option` in general changes, and an interface that never returns `null` never pays for the
test.

### What checks it

`scripts/extern-test.sh` compiles `examples/extern-tests/` and diffs a golden, then runs
`scripts/extern-check.mjs`, which drives the compiled module against
`contract/fixtures/js-extern/chart-lib.mjs` -- a recording fake of an npm chart library -- and
compares the call trace with `contract/fixtures/js-extern/vectors.json` through the contract's own
`compareTrace`. The vectors were produced by executing a reference emission against the fake rather
than by transcribing one, so what the golden pins is what was emitted and what the vectors pin is
that it does what the reference does. A recorder is what makes the distinction a descriptor is
ABOUT into the thing being measured: against a real library, `new Chart(el, cfg)` and a
`Chart(el, cfg)` factory both produce a chart.

## Written JavaScript expressions (`js!{}`, stage 4)

A program writes a JavaScript expression in Rust source with the `js_macro::js` function macro, and
the Rust bindings it names are captured.

```rust
pub fn post(url: &str, content_type: &str, body: &str) -> JsValue {
    js! {
        globalThis.fetch(url, {
            method: "POST",
            headers: { "content-type": content_type },
            body,
        }).then(r => r.text())
    }
}
```

This is the sibling of `#[js_extern]` and not a replacement for it. A declaration names an interface
that already exists and is worth naming; a block is for the value that has no name, and the case
that produced it is above. `fetch(url, init)` takes an options object holding
`{ method, headers: { "content-type": .. }, body }`, and the value model makes a `#[repr(C)]` struct
an object keyed by FIELD NAME. `content-type` is not a Rust field name, and no `#[js(rename)]` fixes
it, because the object is a VALUE rather than a declaration. Before this, `topcoat-dom` declared one
host function whose whole job was to hold that object.

### The block is one expression, and it is not validated

`js!{}` produces a JavaScript expression, not a statement list. A `;` outside every bracket, or a
statement keyword at the outermost depth, is a spanned error; inside a function body written in the
block they are ordinary statements. What is checked is that, that every bracket balances, that no
string, template literal, comment or regular expression is left open, and that no name collides with
an emitted capture slot.

**The block is NOT checked to be valid JavaScript.** There is no parser in the macro and none in the
backend, so a malformed block is a syntax error in the browser rather than a spanned error in Rust.
That is an accepted trade-off, taken deliberately: the alternative measured was `boa_parser`, which
at this pin drags `boa_ast`, `boa_interner`, `boa_macros` and `icu` data for a question that needs
no grammar. If a parser is ever vendored into `contract/vendor`, a build step can parse every
emitted block and report at build time; the analysis below does not change either way.

### What is a capture, and what is not

Free variable analysis is a LEXER, `js-macro/src/lexer.rs`, with no dependency. It skips the four
places an identifier is not an identifier -- strings, template literals (which nest through `${}`),
comments and regular expression literals -- counts bracket depth, and knows the binding forms:
`let`, `const`, `var`, function parameters, arrow parameters, a function's own name, `catch`, and
`for` heads. Everything else identifier shaped is a capture, except:

| not a capture | example |
|---|---|
| a reserved word | `typeof x` |
| one of four host names | `globalThis`, `undefined`, `NaN`, `Infinity` |
| a property after `.` or `?.` | `r.text` |
| an object literal key | `{ method: "POST" }` |
| a name the block binds, where it binds it | `r => r.text()` |

**The failure direction is chosen and it is the loud one.** An identifier the lexer cannot classify
is reported FREE, becomes a real Rust identifier in the expansion, and is therefore a spanned
"cannot find value" from rustc rather than a capture that silently went missing.

A name the host supplies is reached through `globalThis`. That is a rule and not a shortcut around a
builtins list: a list would go stale against whatever the page loaded, and going through
`globalThis` makes every host name a block depends on greppable, which is what keeps "a free
identifier is a Rust binding" total.

**The object literal shorthand is a reference**, so `{ body }` is a capture. It is written out as
`{ body: <slot> }` rather than replaced in place, because replacing the token would have renamed the
key.

**Scopes, not a set of names.** `r => r.text()` needs `r` bound inside the arrow and free outside
one, so a bound name is bound over a region. A scope opens at `) =>`, at `ident =>`, and at the `(`
of a `function`, a `catch` or an object method shorthand; it ends at the close of a block body, or
for an expression body at the next `,`, `;` or non ternary `:` at the same depth.

**Regular expression against division: the lean is toward division.** JavaScript answers this from
the parser's state and a lexer has only what came before, so a `/` after something that ENDS A VALUE
divides and after anything else opens a regular expression. The genuinely ambiguous token is `}`,
which ends a value when it closed an object literal and does not when it closed a block, and it is
counted as ending one. Both mistakes are possible and they are not equal: reading a regular
expression as a division lexes the pattern as code and its identifier shaped pieces come out FREE,
which errors; reading a division as a regular expression swallows the code up to the next `/`, and
an identifier inside it disappears without a word.

### The two ways to write a block

A block is ordinarily written as tokens, and the macro reconstructs the JavaScript from them.
Rust's own lexer runs first, so a backtick never reaches the macro (`error: unknown start of token`)
and comments are gone by then. A block that needs either is written as a single RAW string literal,
whose contents are taken verbatim:

```rust
let greeting: JsValue = js! { r#"`hello ${name}`"# };
```

A plain `"..."` is an ordinary JavaScript string expression, which is what it looks like, so the two
forms are unambiguous. The reconstruction keeps ADJACENCY, which `Spacing::Joint` records: two
punctuation marks written together stay together and two written apart stay apart, so `?.` stays an
optional chain and `a - -b` does not become a decrement. Nothing else needs a space, because an
identifier or a literal after punctuation can never lex as part of it.

### The block descriptor

```text
rcgjs.js.<version>.<slots>.<text>
```

for example `rcgjs.js.1.2.fetch(_$js0,{body:_$js1})`. `<slots>` is how many captures the block takes,
which is also how many arguments the marker call carries. `<text>` runs to the end of the section and
so needs no length, because the three fields before it hold no `.`; it is escaped to one line, with
a backslash spelling `\\`, `\n`, `\r` and `\0` -- the last because rustc refuses NUL in a
`link_section` outright, and the rest so the section stays greppable in a fixture and readable in an
error message. The escape is reversed exactly, so nothing about the JavaScript is normalized in
transit.

The prefix distinguishes the three markers this backend intercepts: `rcgjs.tc.` is `view-abi`,
`rcgjs.ext.` is a `#[js_extern]` declaration, `rcgjs.js.` is a block. Decoding is total and the
version is the first field read, so a block this backend does not understand is a zombie naming the
block rather than a guess.

The format lives in one file, `js-macro/src/block.rs`, which the macro compiles and the backend
includes with `#[path]`, for the reason `js-extern-macro/src/descriptor.rs` does: a proc-macro crate
can export nothing but proc macros, and two copies of a format is how a format drifts.

**The slots are already substituted when the section is written.** The analysis lives in the macro,
which is where the spans are and so where an error can be reported; carrying the capture NAMES
instead would mean a second lexer in the backend to find them again, and two lexers that disagree is
a miscompile. What the backend receives is text with holes at fixed names.

### The transport and the emission

The macro writes a marker function inside the expression, carrying the descriptor in its
`link_section`, and calls it with the captures:

```rust
{
    #[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.js.1.2....")]
    #[inline(never)]
    fn __topcoat_js_block<__JsBlock, __JsCapture0, __JsCapture1>(
        __js_capture0: __JsCapture0,
        __js_capture1: __JsCapture1,
    ) -> __JsBlock { ::core::unreachable!(..) }
    __topcoat_js_block(url, body)
}
```

The captures route through real identifiers, which is what makes an undeclared one rustc's error
rather than the macro's. The marker is generic so that a block takes captures of any type and
produces a value of whichever type its context infers; `js!{}` in statement position therefore needs
an annotation (`let _: () = js!{ .. };`), which is the price of not naming a return type.

The call is intercepted in `abi.rs`'s `codegen_call`, in the arm after the `#[js_extern]` one and
ahead of `is_foreign_item`, for the same reason: the marker is an ordinary Rust function with a body
and would otherwise resolve. `js_block.rs` decodes the section, lowers each argument with the
ordinary operand path, and emits an arrow that is called immediately:

```js
((_$js0, _$js1) => globalThis.fetch(_$js0, { body: _$js1 }))(url, body)
```

An arrow rather than a textual splice of the argument expressions. An argument is an arbitrary
expression: splicing would evaluate it once per occurrence, in whatever order the block happens to
name its captures, and would need the backend to print an expression to text and re-parenthesize it.
Binding is the operation a capture wants and JavaScript already has it. A block with no captures
needs no arrow and gets none.

### What a block costs

The text is `jsast::Expr::Raw`, and `minify.rs`'s local pass refuses to rename anything in an item
holding one, because it cannot see which names the text mentions. **A function containing a `js!{}`
block therefore keeps its long local names.** That is a real cost with a known fix -- hoisting a
block into an item of its own would confine it to that item -- and it is not paid until a block is
written.

The link time item rename is textual, and its soundness argument named the emitted text as having no
regular expression literals, no template literals, no single quoted strings and no block comments. A
block may hold all four. The pass still substitutes only an exact match against the item table, and
an item name carries a `$h` hash infix or is `$a` from the pass itself, so a block would have to
spell one to be touched.

### What checks it

`js-macro`'s own unit tests are the lexer's: strings, template literals and their nesting, comments,
regular expressions against division, parameters and their defaults, declarations and destructuring,
`for` heads and `catch`, property keys against ternaries and `case` labels, the shorthand, bracket
nesting, and every refusal.

`examples/emit/21_js_block.rs` is the end to end fixture, with a byte for byte golden over the
emitted JavaScript and its stdout under node: a block with no captures, numeric captures, the
unspellable key, a capture named twice, the shorthand, an arrow parameter that is NOT a capture, and
a template literal through the verbatim form. It is compiled against the real `core` (`.core`) with
`js-macro` built for the host, which is what `emit-test.sh`'s `NN.macro` companion is for.

`examples/async-tests/06_procedure.rs` is the motivating case, and it is not run here: it needs the
Topcoat framework's `topcoat_runtime_macro` and `topcoat_dom`, which are not part of this
repository, so `scripts/async-test.sh` reports it as SKIP. Its checks stay in
`scripts/async-check.mjs` and run when the fixture is named and built: they install
`globalThis.fetch`, record the init object the compiled program built, and assert the method, the
media type under the header name Rust cannot spell, and the argument array.

## Effect-summary interpretation

The effects corpus evaluates available JIR-R callees using their actual bodies and the caller's heap. A single operation budget covers the whole call tree. An explicit frame stack preserves caller values and cells across calls and bounds nesting at 128 frames without using host recursion. Return values, aliasing, writes and failures are observed independently of the summaries being checked. The per-object corpus uses one shared type and intern table; missing callees and operations outside the interpreter's declared subset produce explicit skipped runs. Type-shaped call stubs remain available for SSA transformation comparisons, but they are not execution evidence for effect-summary validation.

Interpreter outcomes retain the result or trap, ordered call/drop trace and final mutable pointees on both success and failure. A failing call records its trap before propagation. SSA lowering and the DCE, range and partial-escape corpora share the complete-outcome comparison policy. Matching modeled failures agree only when their traces and pointees also agree. Unsupported operations, unavailable hooks, exhausted budgets and undefined original executions are explicitly skipped; malformed executions are mismatches. JIR-R transformation samples follow the lowered parameter list, including spread and caller-location parameters. These observations do not model all host exception identities, cleanup paths or writes restored before exit.

Negative write claims in the effect-summary corpus use monotonic heap write history. Mutable slot access and storage reconstruction mark the containing heap object; allocation, reads and borrowing existing storage do not. A same-value store or a write later restored still contradicts a no-write claim on an argument's tracked pointee. Writes completed before an unsupported operation, unavailable hook or exhausted budget can also disprove the claim. These runs remain skipped, and a prefix without a write does not establish agreement about the unexecuted suffix. Prefix checks and contradictions are reported separately. This history is separate from differential outcomes: it validates conservative summaries without requiring an optimizer to preserve every unobserved intermediate store. Complete transitive alias graphs remain outside this check.
