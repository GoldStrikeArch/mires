# Verdict: rustc_codegen_mires

> **Scope.** The rounds below were run while the compiler lived inside the Topcoat
> repository. The `view!` lowering, the `dom-tests` suite, the demo app, the parity
> harness and the procedure wire they mention are the Topcoat integration, which stays
> with Topcoat; the counts quoted for those suites are not reproducible from this
> repository alone.

Three rounds, all GO.

- **Stage 0 (spike), 2026-07-29 morning**: proved a rustc codegen backend can lower
  monomorphized MIR to correct JavaScript. 8 golden tests, a browser demo, ~1,400 lines
  of lowering. (Original spike verdict preserved in git history.)
- **Stages 1+2, 2026-07-29**: the language core and the output-quality stack.
- **Stage 1.5, 2026-07-29 evening**: the `{buf, off}` pointer model. See below.

## Stage 1.5: the pointer model — the "R1 risk" is retired

The one problem the original research flagged as having no clean precedent — raw
pointers in a structural value model — is solved and proven. A pointer is a **slot**
(`{buf, off}`, offset in pointee units) with scaled/window/address forms for the byte
views, casts and provenance-free cases; synthetic WeakMap addresses make `is_null`/
`is_aligned`/`fmt::Arguments`' tag bit guarantees rather than accidents.

What that unlocked, all green in `examples/core-tests/`:

- **`core::fmt`**: `write!` with the full format-spec surface (radix, width, fill,
  precision, named/captured args, user `Display` impls) — real formatting machinery,
  template bytes and erased argument vtables included, running as JavaScript (11_fmt).
- **`slice::iter()`** and the adapter chain, `iter_mut`, slices of structs and ZSTs
  (12_slice_iter) — green on the first compile after the model landed.
- **`sort_unstable`** on `[i32]`/`[u64]`/structs at every length — the whole ipnsort
  tree: insertion sort, sorting networks, quicksort partitioning through
  `MaybeUninit` scratch buffers and gap guards (13_sort).
- **`str`**: byte views (the old silent `as_bytes` miscompile is fixed and pinned),
  `chars`/`char_indices`, comparisons, `starts_with`, `get(a..b)` (14_str).
- **`15_ptr`**: the model's own unit test — null/dangling round trips, `ptr::eq`
  across independently-taken pointers, overlap-safe copies, offset arithmetic.

**Surviving zombies in `core`: 5,600 -> 20 (-99.6%).** Suite: **33/33** runtime +
**14/14** emit goldens, green in every flag mode. Minify gzip cuts 32-58%.

Seven backend bugs were found by the test workstream and fixed by the backend
workstream during the wave — including three SILENT wrong answers (`addr()` losing
the element size, whole-aggregate writes through boxed pointers dropped, quicksort
aliasing struct elements through the partition) — exactly the class of bug this
round existed to make unshippable. Hoisted constants shipped but default OFF on
measured evidence (incompressible hash names lose 1-5% gzipped); the real win was
`--remap-path-prefix`, now unconditional.

Honest remainder (documented in `examples/core-tests/README.md`): `&s[a..b]`/
`contains` (an `[AsciiChar] -> str` cast in the panic path), `find` on haystacks
over 8 bytes (`memchr`'s word-at-a-time `align_to`, a deliberate run-time refusal),
`NonNull::dangling` via `Alignment`, `read_unaligned`, and ~5 one-off transmutes.
`alloc` remains the next representation-design round.

## What exists now

A rustc codegen backend (dylib, `-Zcodegen-backend`) plus a standalone structurizer
crate that together compile Rust — **including the real `core` library** — to JavaScript
that reads like handwritten code, with source maps and a minifier.

```
function area(r) {                          // <- real Rust, compiled
  return Math.imul(r.w, r.h) | 0;
}
```

Verified state (one command each, from `spike/rustc-codegen-js/`):

| suite | result |
|---|---|
| `./scripts/test.sh` (17 mini_core + 10 real-core + zombie golden) | 28/28 |
| `./scripts/emit-test.sh` (golden emitted-JS fixtures, each also executed) | 13/13 |
| `cargo test` (backend 70 + structurizer 48, incl. a 720-run CFG property test) | green |
| flag-mode matrix (trampoline / queue-off / scoped-lets / line-comments / source-map / minify / minify=locals) | 28/28 each |

## Stage 1: the language core

- **Zombie system + DCE as one graph** (rust-gpu's model): unsupported constructs record
  deferred errors that fire only if reachable from the program's roots, with
  `required by` chains; unreachable items vanish. The proof: `library/core` compiles
  through the backend in **6.4 s** with **one patch** (panic paths carry static messages
  instead of `fmt`), leaving **5,600 zombies inside core, all silent**. A program that
  reaches one (e.g. `write!`) gets a precise chain from the `core::fmt` internals up to
  its own `rust_entry`.
- **Language surface**: drop glue (recursive, rustc-synthesized), `dyn Trait` with
  array-shaped vtables at rustc's exact slot indices, supertrait upcasts, slices as
  `{buf, off, len}` with shared-buffer subslices, statics (provenance-aware constant
  reading), i64/u64/i128 as BigInt with the full cast matrix, `usize` genuinely 32-bit
  (wasm32 target: correct by construction), saturating float casts, `Math.fround` f32,
  a 166-key intrinsics table, full `#[track_caller]`, representation-aware `transmute`.
- **Real-core test suite**: Option/Result combinators and `?`, `Ordering` and derived
  `Ord`, the whole Range-iterator adapter chain, slice indexing and patterns,
  wrapping/checked/overflowing/NonZero arithmetic, `mem::swap`/`replace`/`Drop` order,
  `&dyn`/`&mut dyn`, closures through `impl Fn` bounds, and a `#[track_caller]` panic
  resolving to the user's `unwrap()` line.

## Stage 2: the output-quality stack (the jsoo playbook, ported)

- **Structurizer** (standalone crate, 48 tests incl. a randomized dual-interpreter
  property test): dominator-tree relooping with `shrink_loops`, DTree switch synthesis
  (`===`/range/`(x-lo)>>>0<len` forms, sign-biased value ordering), dispatch fallback for
  irreducible graphs. Integration alone cut emitted bytes **41.8%**.
- **Expression queue + destination passing**: MIR's three-address temporaries fused into
  expressions across block boundaries; declared locals **-83%** (5.9x, jsoo's promised
  3-5x), emitted lines -35%, `while`-loop recovery. Queue-off reproduces prior output
  byte-for-byte for bisecting.
- **Readable names**: locals and params from MIR `VarDebugInfo` (empirically verified to
  survive `-Cdebuginfo=0` and inlining), item names as sanitized def-paths with
  deterministic hash suffixes.
- **Source maps v3**: statement-granularity, byte-identical JS guaranteed (a printing
  no-op marker, not an AST wrapper), VLQ-validated by an automated decoder, working
  across crate boundaries (stepping into `core/src/iter/range.rs` from an rlib),
  `names` table for original Rust identifiers, `ignoreList` blackboxing for glue,
  DevTools recipe in the demo.
- **Minifier**: whole-program item rename over the linker's own table plus a
  frequency-sorted local renamer and compact printer. **Gzip: -31% to -58%** vs readable
  (`scripts/measure-size.sh` table in CONTRACT.md). Deterministic, `node --check`-clean,
  exports preserved.

## Known boundaries (deliberate, documented, zombie-guarded)

1. **Raw pointer arithmetic** — the dominant blocker: `slice.iter()` and everything on
   it (`sort`, `contains`, `copy_from_slice`), `split_at`, `str` beyond len/printing.
   The designed fix is the `{buf, off}` universal pointer slot (stage 1.5 decision,
   deferred by plan).
2. **`format!`/`Debug`/`Display`** — blocked on (1) plus the one core patch; `&dyn
   Display` vtables would otherwise drag all impls into every program.
3. **`alloc`** (Box/Vec/String) — deferred to the representation-attributes design
   round, per plan.
4. f16/f128, SIMD, real threads, `TypeId` equality.

## Measured next wins (evidence from this round)

1. Shared-constant hoisting: repeated `#[track_caller]` location literals are up to 39%
   of a minified small program.
2. `{buf, off}` raw pointers -> unlocks most of the OUT list above.
3. Source maps under minify (rename through the `names` table).
4. Then: stage 3 — the `Template{html,holes}` IR and the dom-expressions target.

## Operational notes

- The sysroot must be rebuilt when emit-affecting flags change; `test.sh` now does this
  automatically via `build/sysroot/.js-args`.
- Emit-affecting flags must match across every crate in a program (CONTRACT.md).
- The toolchain pin is nightly-2026-07-27; the maintenance playbook (daily bump cron,
  commit-hash pinning) is designed and not yet automated.

# Stage 3 checkpoint — gate G1: the counter island (2026-07-30)

**VERDICT: GO.** The compiler and the framework meet: one `view!` body in
`demo-app/island/counter.rs` produces both the server HTML (hydration keys, `<!--$-->`
markers, signal seeds) and the compiled client module, and the served page hydrates with
**zero DOM mutations** and updates with **one** — upstream-identical, marker comments
preserved — with no hand-written island JavaScript anywhere.

## The numbers

- **Suites**: runtime 42/42 (all flag modes), emit goldens 17/17, module 16/16,
  dom 14/14 (every fixture compiled *and* executed against the contract's recording
  stub), backend+structurizer+view crates 265, root workspace 1643 default /
  1728 all-features, hydration parity 39/39 with two negative controls failing loudly.
- **The counter island module**: 3,054 bytes raw / 948 gzipped. The vendored runtime
  (solid-js/web + core, one reactive graph): 23,971 raw / 9,106 gzipped, pinned and
  reproducible.
- **L2 trace parity vs the pinned reference compiler** (babel-plugin-jsx-dom-expressions
  0.40.7): family 01 MATCH; every other compared family's differences fully attributed
  in `examples/dom-tests/deltas.json` (zero stale rules); standing structural deltas are
  anchor placement, `delegateEvents` placement, and match-arm memo (each documented with
  its reason).
- `library/alloc` compiles through the backend (Box/Vec/String/format!); core zombies
  7, alloc 22, every one grouped and reasoned in CONTRACT.md.

## What exists now that did not at stage 2

The dom-expressions contract re-extracted from pinned upstream into driven fixtures and
an executable trace harness (`contract/`); ESM emission with byte-identical item text
across modes; the `TemplateData` ABI (v3) and its `link_section` marker channel; the
`WriteDom` third emitter over the un-forked grammar; compile-time `KeyPlan` ordinals
with render-time server key numbering matching solid's `getContextId` encoding (264
oracle cases); `#[island]` with `<mires-island>` mounts, seeds, an import-map loader
and coexistence with the legacy runtime; the retype-once heap model behind `alloc`; and
the SSR↔client hydration parity harness whose negative controls prove it can fail.

## Honest OUT list at this checkpoint

Components/`#[component(client)]`,
`#[procedure]` from compiled code, keyed `for` (`each` renders index-keyed lists),
nested islands, client-side text conversion for non-primitive hole values (a `&&str`
hole inserts its slot record), `js-names=mangled` with core+alloc (crate-dependent
symbols; readable names are the tool), memo for `match` arms (deliberate: a pattern
test cannot be split without matching twice), per-island chunking and lazy hydration
(stage 4).

# Stage 3 checkpoint — gate G2, and the Stage 4 tracks (2026-07-30)

**VERDICT: GO on both.** The full `view!` surface compiles through the DOM emitter, and the
chosen Stage-4 tracks — the string-tag representation, `#[js_extern]` with `.d.ts` emission,
per-island chunking with lazy hydration, and the two-tier builder — are landed and measured.
The audited basis is `contract/G2-FINAL.md`; every number below is from suites that ran.

## The milestone apps

Three compiled islands serve from `demo-app`, none with a line of hand-written island
JavaScript: the **counter** (hydrates at zero DOM mutations, updates at one), the **search
island** (`#[procedure(serde)]` over the typed wire, debounced input, reactive result list),
and the **dashboard** — the original Stage-4 milestone shape: an SSE tick feed into an island
whose `EventSource`, `querySelector` and chart calls are five `#[js_extern]` declarations
(three at global scope, importing nothing), with `#[repr(C)]` Rust structs crossing as the
chart's config and update payloads.

## The numbers

- **Suites**: runtime 47/47 across all 9 flag modes (`js-names=mangled` 33/47, pinned:
  crate-dependent symbols cannot carry core+alloc); emit 20/20; module 19/19; dom 22/22;
  extern 4/4; async 7/7; dts 2/2 (tsc-checked); backend+crates cargo 420; root workspace
  1789 default / 1886 all-features, clippy and rustdoc clean; parity 217 checks across 4
  live fixtures with every negative control failing on its named check; 11 budget subjects
  in band; contract drift 15/15 steps zero.
- **L2 trace parity vs the pinned reference compiler**, now a committed and enforced gate
  (`l2-status.json`, regression AND improvement both fail): 1 MATCH, 13 DIFFERS with every
  line attributed to a named cause, 2 EXCLUDED, 2 GAP (SVG and custom-elements, specs banked).
- **Sizes**: dashboard page 21,077 B gzip total, of which the shared vendored runtime is
  9,140; per-island chunks 845–2,403 B gzip; a counter-only page pays 919 B — 35% less than
  the pre-chunking single module.
- **The representation**: string-tagged enums shipped as the default on measurement
  (+2.3% worst-mode gzip, dom suite −0.5%, readability improved; mixed enums keep
  `{TAG:"…"}` objects so `&mut` writes hold).

## What the discipline caught across these waves

Eight silent miscompiles (minifier item/field collision, reactive sinks handed accessors,
boxed returns returning the box, transparent-wrapper array places, missing component bodies,
`Sig` rebuilt field-by-field, `char` as code point, coroutine discriminant as `num(0)`), a
`ControlFlow<Infallible>` constant-switch bug reached by every `?`, the procedure client half
stripped under every cfg, and three harness checks that were passing by being unable to fail.
Every one is pinned by a regression test.

## Honest OUT list at this checkpoint

Keyed `for` reaches identity-preservation but not solid's `<For>` equivalence — decided
permanent with evidence (an eager Rust loop clones rows per render before any reconcile; the
cached-node path keeps stale text and both fix shapes are unsound); the movers list ships
unkeyed with the behavior pinned. SVG and custom-element families (specs banked: an ABI field,
`cloner_key`, the synthetic `<svg>` wrapper plus walk re-rooting, which fail silently if done
separately). `js!{}` designed, not built (lexer-not-parser; the minimal form is an expression).
Nested islands refused with a spanned error. f64 serialization through serde (byte-punning,
architectural). Executor async is handler/effect-only by contract (§14: island setup must be
synchronous). Upstreaming beyond `crates/mires-dom` (the grammar move and `#[island]`
beside `#[shard]`) awaits a backend distribution story; `topcoat-cli` is unpublishable while
it path-depends on `mires-build` (recorded in its manifest).

## Corrections to the earlier checkpoint above

Family 07's props equivalence is no longer undefined — defined as eager props destructured at
the callee's boundary, proven by the nested island fixture at 62/62. `#[procedure]` from
compiled code is real (the search island), typed-struct serde included; `.d.ts` emission and
the two-tier builder (`topcoat client setup`; user crates stay on stable) exist. Client-side
text conversion landed as the `text` marker with primitive fast paths.
