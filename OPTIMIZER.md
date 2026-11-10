> Recovery baseline: this document was imported from the September 7 v2 snapshot.
> It describes historical mechanisms and limits, not completed October recovery or production qualification.
> See `docs/V2-RESTORATION.md` and the current `VERDICT.md`.

# Generic rustc_codegen_js optimizer

This document records the current generic optimizer architecture for the Rust-to-JavaScript spike. The optimizer is opt-in and defaults to `off`. THE optimizer is the JIR-lane pass pipeline: producer-baked passes plus the link-step passes over JIR-R (`jsc-link/src/opt/`) and the representation planner (`jsc-link/src/plan/`). The v1 link-time **text** optimizer that once rewrote printed baseline JavaScript was deleted in Phase 5 (W5h) after the jir lane reproduced all nine of its size receipts within 2%; its record is the "History" section below.

## Contract

In `on` mode, the optimizer may change the internal representation of an ordinary Rust `Vec`, `String`, or aggregate that contains them. It must preserve Rust-visible length, capacity, mutation, move, alias, panic, and drop behavior for every operation it lowers. A rejected proof keeps the existing representation for the complete connected region. There is no runtime deoptimization or speculative representation test. The planner may insert statically selected `Convert` adapters at proven cut edges.

Exported candidate slots stay on the baseline carrier. Native shapes are private until the linker can emit and verify a stable canonical wrapper; publishing a representation row is not itself an external adapter. Exchange across artifacts or between `off` and `on` output remains unsupported unless an explicit host contract says otherwise.

`JsValue`, `JsString`, `JsArray`, `repr(C)` values, browser handles, Topcoat's view and DOM ABI, hydration values, owner state, and owner-aware async policy are pinned boundaries. Shared compiler passes may remove code around them, but cannot change their representation, ordering, or scheduling behavior.

Rollup and Terser remain optional packaging tools. The compiler performs typed lowering, reachability, conservative local simplification, and its own requested syntax and item-name minification. Optimization decisions do not use workload names, FingerprintJS signals, application annotations, source paths or spans, emitted-name patterns, or browser-specific algorithms. Exact compiler-owned diagnostic-item identities mark supported standard-library contracts. All compiler implementation remains safe Rust.

## Modes and build interface

The optimizer lives on the jir lane (`-Cllvm-args=js-lane=jir`). The pair `js-lane=jir js-opt=on`
was refused through Phase 2 — two rewriters may not own one item — and Phase 3 lifted the refusal
by re-landing v1's rewrites as passes over JIR-R (`jsc-link/src/opt/`). Phase 5 (W5h) deleted the
v1 text optimizer, so on the **baseline** lane `js-opt=on` now means: the producer-baked passes
(sroa, ast-dce, future-poller, baked into each object's printed text) plus link-dedup — no
link-time rewriting of printed JavaScript happens anywhere. Baseline `on` output is byte-identical
to `off` plus dedup. On the jir lane, `sroa` is the partial escape analysis (`opt/partial_escape.rs`,
which subsumes v1's local split, copy-of-literal and literal-into-`bytes_to_int` rewrites),
`ast-dce` is the SSA builder's `fold_runtime_checks` plus `opt/dce.rs` (constant switches,
unreachable blocks, dead ops, trivial block parameters), `future-poller` is the lowering's own
monomorphic poller, `native-representations` is the representation planner (`plan/representation.rs`: the
value-flow/legality graph, the min-cut carrier solve, and the transactional commit of
`HostString`/`StringBuilder`/`NativeVec`/`NativeBox`/`SliceView` carriers with `Convert` adapters), and `link-dedup`
is unchanged and runs on the printed text of both lanes. Two link-only options,
`js-jir-flat-abi=on|off` (default on), turns off the flat direct-call ABI the lane plans, for
bisecting, while `js-jir-typed-storage=on|off` (default off) enables the narrow proven
exact/reserved numeric-`Vec` lowering described below.

`js-opt=report` runs no rewrite on either lane, and on the jir lane it still reports what
`js-opt=on` would have done. Each pass runs over a *clone* of the function, its stats are kept and
the rewritten clone is dropped (`opt/pipeline.rs`); a clone that fails to verify is reported as
"would not have applied" rather than declining the item, because a decline is a difference `off`
does not have. The flat direct-call ABI is planned the same way — the plan is made and reported,
then emptied before anything is rewritten, so no callee changes shape and no spill register is
added to the program. The current lane sidecar `<out>.lane.json` (schema 15) says both numbers:
`passes.would_apply` beside `passes.applied`, and `abi.applied`. Flat ABI parameter widths and
eligible field-only product-result widths are numeric lane counts; a result width greater than
one means lane 0 is returned normally and lanes 1..N use collision-safe `$rN` spill bindings.
The `physical` block, introduced in schema 8, names the selected `ValueShape` and `CallAbi`,
canonical-wrapper policy, aggregate `StoragePlan`/`DropPlan` counts, and exact compiler-link
economics. Schema 13 additionally reports each numeric storage candidate's constant initial
capacity, proven maximum length, proven exact final length, terminal reason, and selected state.
Schema 15 replaces the ambiguous slice-iterator count with `slice_iter.would_lower` and
`slice_iter.final_emitted`, both globally and per item. Report mode can therefore publish an exact
opportunity while reporting zero emitted cursors, and an item rejected by witness checking retains
its opportunity count while contributing zero to final output.
`storage_mode` is `exact-opt-in` whenever the executable override is active and `report-only`
otherwise; decision rows remain the authority for whether any TypedArray was emitted. Each
final-JavaScript loop-allocation site also reports its originating JIR value and definition, Rust
type, selected physical shape, and the first boundary that forced materialization.
Its representation block also
separates `planned`, `validated`, and `final_emitted`, and each item reports fresh literal sites
from the final lowered JavaScript AST. Under `on` the two pass blocks are
equal, under `report` `applied` is all zero and `abi.applied` is false, and under `off` both blocks
are zero and `abi` is `null`. `report`'s `would_apply` is measured over functions no earlier pass
touched, so it reads a little lower than `on`'s: under `on`, `sroa` sees the flattened ABI and
`ast-dce` sees the SSA builder's folded runtime checks. The byte identity the mode promises is
checked by `scripts/jir-report-test.sh` (`29_rc` and `60_virtual_struct_merge`, `off` against
`report`).

The options below are still parsed by the backend from `-Cllvm-args` and travel to the link step
in the manifest:

| option | meaning |
| --- | --- |
| `js-opt=off` | Preserve baseline lowering and skip optimizer and typed late-IR analysis. Serialized objects carry only a schema-compatible unavailable marker. This is the default. |
| `js-opt=report` | Run analyses and record decisions, but emit the baseline declaration and baseline references. Deterministic emitted JavaScript is byte-identical to `off`; rlibs differ because `report` carries analysis metadata. |
| `js-opt=on` | Apply proven AST, async, executable late-IR, and native representation rewrites. Native representation changes commit only as complete atomic regions. |
| `js-jir-typed-storage=on|off` | Link-only experimental override for exact/reserved numeric `Vec` storage. It defaults to `off`, requires `js-lane=jir js-opt=on`, and never changes the sysroot key. |
| `js-opt-passes=all|none|LIST` | Developer-only attribution mask. The registered order is fixed as `sroa`, `ast-dce`, `future-poller`, `native-representations`, then `link-dedup`; the list only selects passes. |
| `js-opt-warnings=on|off` | Enable or disable warnings for reachable rejected workspace spans. The default is `on`. |
| `js-opt-report=PATH` | Write the deterministic whole-program size and optimization report. |
| `js-workspace-root=PATH` | Define which source paths may produce warnings. Other misses remain in JSON only. |

`js-opt` and the pass mask are emit-affecting, so the client, dependencies, and JavaScript sysroot must agree. The typed-storage override is deliberately link-only: one set of already compatible `js-opt=on` objects can be linked both ways. `jsc-build` exposes the emit-affecting settings through `BuildConfig::optimizer`, `optimizer_passes`, `optimization_warnings`, `optimization_report`, `workspace_root`, and `rust_opt_level`. The published report is returned as `Artifacts::optimization_report` and normally uses the `<program>.opt.json` suffix.

```rust
use jsc_build::{BuildConfig, OptimizerMode, OptimizerPass, RustOptLevel};

let artifacts = BuildConfig::new("client/app.rs")
    .optimizer(OptimizerMode::Report)
    .optimizer_passes([OptimizerPass::NativeRepresentations])
    .optimization_report(true)
    .optimization_warnings(true)
    .rust_opt_level(RustOptLevel::O1)
    .render()?;
```

The explicit Rust optimization level is applied to both the client and its sysroot. No higher Rust optimization level became a default as part of this work.

## Implemented emission changes

### Scalar replacement of aggregates

A Rust value is a JavaScript object or array here, so copying one, or handing one to an operation that reads it field by field, costs an allocation. In `on` mode each lowered item drops the aggregates that only exist to be taken apart again.

Three rewrites, each exact by construction rather than by analysis. A copy of a freshly built array literal is that literal, because nothing else can hold it. An operation that binds its operand to a parameter and reads only `p[0]`, `p[1]`, ... once each, in the order JavaScript evaluates them, reads the array literal's element expressions instead; this is the shape the byte conversions emit, and it keeps effectful elements in their positions. A function-scoped local whose every assignment is one object literal over the same fields, and whose every other mention is one of those fields, becomes one local per field, assigned in the literal's own order.

A local is left alone when it is used whole, indexed dynamically, read at a field no literal writes, shadowed by a parameter, reachable from verbatim JavaScript, or assigned a literal that mentions it. Only locals declared directly in the function body are considered, so a split never moves a binding out of its block.

The finished-JIR rounds may also forward compiler-local alias chains. JIR SSA proves that the source definition dominates the alias. A direct source must be an unassigned parameter or a local whose JavaScript slot is written exactly once; a field path must still start at a fresh object literal and end in a field that is never replaced. The alias itself must have one source. The rewrite substitutes the already-evaluated source slot, not its defining expression, so it cannot duplicate or move evaluation and object mutation remains visible through identity. A reassigned source or replaced field keeps its snapshot alias. Shadowing and verbatim JavaScript decline the complete alias rewrite.

The pass trades syntax for allocation: splitting a local grows the printed body a little and shrinks the minified one, because the object literal is gone and the new locals rename. It is `sroa` in the attribution mask, and it is worth the most where rustc's own MIR inliner has run over the caller (`JSC_RUST_MIR_INLINE=on`), which is what puts an aggregate and the code that takes it apart in one body.

### Conservative AST simplification

Each lowered item is simplified before JavaScript references are collected in `on` mode. Removing a reference can therefore remove the now-unreachable `core`, `alloc`, formatting, panic, libm, or application item at the normal link-time DCE step.

The pass selects literal `if` and conservative literal `switch` branches, removes literal-false loops, folds finite primitive literal operations, truncates after unconditional termination when no later binding can affect scope, and removes unused declarations only when their initializer is syntactically inert. It retains block scope when selecting a branch and refuses changes that can disturb function hoisting or a `let`/`const` temporal dead zone.

Calls, getters, computed properties, allocation, assignment, coercion, raw JavaScript, unknown identifiers, proxies, and dynamic scope are effectful. `NaN`, infinity, mixed Number/BigInt operations, loose equality, and unsupported switch fallthrough also remain unchanged. The pass iterates only while the AST becomes smaller.

`report` performs the same analysis on a clone, records the candidate, then emits and references the original item. This is what preserves baseline output and reachability.

### Physical products, results, and slice lanes

The ABI planner flattens eligible private direct-call tuple, array, and small-struct parameters into scalar fields. It carries these lanes through loop block parameters, direct-call chains and SCCs, and results forwarded directly into another prepared call. An internal, field-only product result may return lane 0 normally and write the remaining lanes to generated `$rN` spill bindings. The caller reads those bindings immediately; virtual-result forwarding avoids constructing a temporary semantic aggregate when the next consumer only projects lanes.

The same machinery admits a deliberately narrow tagged-product tranche for two-variant `Option`, `Result`, and ordinary enums. Their physical form is `tag, payload...`; it is selected only for a hot direct producer-to-leaf chain whose consumer uses the scalar payload and cannot observe the enum object. Exports, function pointers, vtables, indirect calls, host calls, and true escapes keep the canonical materialized ABI. Fixture 98 covers fixed products through loops, chains and mutual recursion, tagged products, canonical wrappers, indirect calls, source maps, and optimizer-off/on runtime parity.

An eligible slice parameter uses the same call plan but is lowered physically as `buf`, `off`, and `len`. The semantic JIR value remains a slice, so the range pass can propagate metadata through subslices, fixed chunks, `off + index`, and loop-carried lengths before JavaScript lowering. Member folding then emits direct buffer indexing without constructing a fat-pointer record between prepared callers. Fixture 97 proves 11 bounds-check removals, eviction of the owned four-item panic/location subgraph, and zero final-JS loop allocations in the selected slice body.

Exports and non-call references retain a canonical materialized entry point while prepared direct callers target at most one collision-safe private `$n1` body. Indirect calls, vtables, unresolved references, suspension, unsupported field use, and recursion shapes the verifier cannot prove stay canonical. Spill and version names use program-wide collision-safe allocators. Caller and callee changes are transactional; if either side declines, the linker retries with the canonical ABI. Fixtures 91, 93, and 98 plus `scripts/flat-result-test.sh` protect product results, loop-carried and recursive/direct-call lanes, tagged products, source maps, exports, and function-pointer wrappers. Fixture 97 and `scripts/slice-lane-test.sh` protect slice lanes, range witnesses, transitive panic-subgraph removal, and canonical slice wrappers.

### Exact byte-slice iterator cursors

Finished JIR can replace the exact standard
`Enumerate<Take<SliceIter<u8>>>` and `Enumerate<Skip<SliceIter<u8>>>` driver shapes with a
loop-carried `usize` cursor. JIR schema 9 records `Slice`, `Enumerate`, `Take`, and `Skip` type
identities from rustc diagnostic items; paths, emitted names, source spans, and workload identities
do not select the rule. The custom sysroot patch adds diagnostic items to the two bounded adapters.

The matcher requires the exact `u8` construction chain, start/end pointers and slice metadata from
one source, the pinned direct-value or fresh mutable-borrow driver CFG, a complete loop SCC, and
non-escaping adapter state. All loop exits must reach one parameter-free block. Intermediate exit
blocks may perform only structural reads or writes of the dead adapter state, so an ordinary user
`break` is legal while calls, arithmetic, retained state, and observable stores decline.

`Take` checks `cursor < len && cursor < limit`; `Skip` starts at `limit`, checks `cursor < len`, and
uses `cursor - limit` for `Enumerate`'s index. Both borrow `source[cursor]` and increment by one.
The existing DCE owns deletion of the unreachable adapter graph. A separate
`SliceIteratorCursor` witness records each adapter, source, length, limit, cursor, guard, indexed
borrow, enumerate index, preheader, backedge, body, and exit, then requires an exact one-to-one
match before slice-forwarding changes the pointer spelling. The matcher reserves each complete
rewrite SCC and its preheader, so selected plans cannot overlap. Any verification or witness
failure discards the rewritten item and preserves the baseline declaration.

Fixture 105 and `scripts/slice-iterator-test.sh` pin both adapters, empty and short inputs,
out-of-range limits, a nonzero subslice, and early `break`. `u16`, `i8`, reversed adapter order,
and observed remaining state are hard declines. A fault targeted specifically at
`SliceIteratorCursor` verifies item-level rollback with `proof.witness`.

### Exact/reserved numeric storage

With `js-jir-typed-storage=on`, a private global-allocator `Vec<T>::with_capacity` may keep its
semantic `{buf, len, cap}` carrier while replacing only `buf` with an exact numeric TypedArray.
The supported element set is `u8`, `i8`, `u16`, `i16`, `u32`, `i32`, `f32`, and `f64`; the
32-bit target's pointer-sized integers use the matching 32-bit forms. Constant evaluation applies
each integer cast with its source and destination width and signedness before interpreting a
nonnegative capacity.

The length proof follows only one acyclic chain of blocks joined by unconditional `Goto` edges.
It starts constructors at zero and tracks `push`, `clear`, `remove`, and owned `extend`; a branch,
cycle, overflow, invalid removal, or opaque mutation forgets the proof. Selection requires both a
constant initial capacity and a proven maximum. The TypedArray is allocated once at the greater of
those values, while `len` and the unobservable logical `cap` remain separate. The executable
operation set is deliberately smaller than ordinary `NativeVec`: push, clear, length, emptiness,
indexing/mutation, POD drop, and an immutable slice whose only terminal users are selected direct
scalar-slice calls.

Element drop, custom or unknown allocation, capacity/layout or address observation, raw access,
retained or mutable views, owned identity/copy, uncertain maximum length, removal/extension in the
executable path, and host, indirect, vtable, export, return, or otherwise unadapted boundaries all
decline. The generic same-element `Vec` bodies and default output are untouched. Fixture 104 and
`scripts/storage-plan-test.sh` pin the facts, hard declines, logical capacity, integer truncation,
scalar slice handoff, and off/report byte identity. Its compressed output grows under the opt-in,
and the complete FingerprintJS request build selects zero candidates, so no release profile enables
this path.

### Native Box carrier

`NativeBox` is available only for compiler-owned default allocation and drop semantics. Dropping a native box performs the exact proven pointee drop glue, including structural field drop, and does not retain baseline allocator-release machinery when release is proven unobservable. A user-defined destructor on the boxed pointee, custom or unknown allocators, raw/address observation, missing structural glue, and unsupported ownership keep the complete region baseline. Exact leaf glue reached through structural fields remains legal.

Fixtures 89, 90, and 94–96 protect trivial and structural drop, nested ownership, early returns, mixed direct/indirect callers, panic-abort, and host-throw paths. Compact FingerprintJS currently has nine legal `NativeBox` representation candidates, but exact compiler-link artifact measurement declines every one as `cost.region-no-shrink`; legality never forces an uneconomic commit.

### Mutation-safe projection forwarding

The interprocedural projection pass may replace a copy of a freshly returned field-only product with the corresponding call arguments only when that copy is the immediately following operation in the same block. It does not forward across mutation or any intervening operation. This restriction closes the X64 hashing miscompile where a later copy of a mutated returned object was replaced with constructor-time scalar values. Fixture 92 and the projection unit tests protect both the immediate positive case and mutation negative case.

### Native host boundary

`js_bindings!` lets safe application Rust declare the existing static host descriptors without writing an `unsafe extern` block. The descriptor set now includes computed-property membership, computed method calls with zero to two method arguments while preserving `this`, and fallible direct get, call, construction, and method operations returning `Result<T, JsValue>`. Fixed shapes use direct JavaScript operations. `Reflect.apply` and slice arguments remain only for a runtime-selected function or arbitrary argument list.

`topcoat-js` provides opaque `JsValue`, `JsString`, and `JsArray<JsValue>` boundary values. `JsString` deliberately uses JavaScript UTF-16 indexing and is not a Rust `String`; `JsArray` is not a Rust `Vec`. `JsValue::erase` and `read` accept only sealed, copyable host representations and validate the JavaScript type and Rust value range before reading. Owning Rust values cannot bypass their representation or `Drop` contract through this API.

A `js_bindings!` parameter is a sink only when the declared type provably carries the same JavaScript value in the baseline and native lanes: the scalars, and `&str` behind shared references. Sink arguments do not mark their value-flow component `HostBoundary`, so a native immutable `String` borrowed as `&str` can commit and is passed as the host string; a mutable `{text, cap, len}` passes `text`. Everything else stays a pin: mutable references (the host can write back through the carrier), owned `String` and `Vec` values (their baseline host value is the struct carrier), slices and `JsArray` (a native `Vec` backing array is capacity-length, not len-length), opaque `JsValue`, view-ABI, `repr(C)`, and raw provenance. Host-call results unpin only for the same sink shapes. The binding item itself remains host. Mixed callers keep the original baseline item; prepared native callers may target one private `$n1` version. A transaction still commits only if every member is proven and the complete reachable program shrinks.

Rust closures can become host callbacks at arities zero, one, and two. Their capture environment transfers to the JavaScript realm, so Rust destructors are not a callback-release mechanism; a resource that needs deterministic release needs an explicit host cleanup operation. Synchronous host throws can be caught as `Result<JsValue, JsValue>` through `try_host` or a fallible binding.

The minimal-shim extractor follows transitive `__rt` member references and is available from `jsc-build`. It changes the packaged runtime surface, not Rust semantics.

### Monomorphic Future poller

In `on` mode, the backend recognizes the async terminal markers used by `topcoat_js::spawn_local`, `topcoat_js::future_to_promise`, and `view_async::spawn`. It passes each concrete future's poll and drop functions directly to a small host task state. The emitted path does not retain the generic `Box`, `Rc`, `RefCell`, wrapper task, or raw-waker-table machinery.

The shared scheduling policy coalesces wakes, prevents reentrant polling, keeps the future pinned in stable task state, drops it before settling a Promise, and preserves the original rejection value. `topcoat-js` performs the first poll synchronously. Topcoat captures the current owner before a deferred first poll and re-enters it for every continuation. A real task yield uses a fresh `MessageChannel`; later Future wakes use microtasks.

`off` and `report` use the safe Rust fallback executor. The fallback is intentionally semantically equivalent, but only `on` substitutes the monomorphic host task state.

The `off` baseline is the current safe executor, not a byte-for-byte reconstruction of the earlier unsafe raw-waker task table. Its contract is no optimizer rewrite plus protected semantic parity. `off` and `report` emit byte-identical deterministic JavaScript; their rlib metadata differs.

### Link-time duplicate item merge

Rust monomorphizes and JavaScript is type erased, so instantiations that differ only in a type parameter print the same body under several names. In `on` mode the link step groups reachable functions by their body, keeps one of each group, and points every reference at it.

Two bodies belong to one group when they differ only in the item's own name, which appears in the declaration and in any self-recursive call. The comparison cuts each body at its own name and compares the remaining pieces, so a body that calls itself never matches one that calls the group's other member. Cross-references keep their spelling, which is why two mutually recursive pairs do not merge. Merging a callee makes callers that only differed in which twin they named identical, so the pass repeats until a round merges nothing.

The surviving item is the lexicographically smallest name in the group, and every other member is merged away unless something outside the item graph names it: a fixed name, an export, a dead-code-elimination root, a declaration whose position is observable, or an item whose native representation late lowering committed to. The survivor takes the union of the group's analysis metadata and unlowered constructs, with the same conflict degradation a duplicate definition of one name already gets. Source maps decline the merge, because an item's map entries are offsets into text a redirected reference rewrites.

## History: the v1 text optimizer (deleted in Phase 5 W5h)

Everything from here to the end of "Measurements" describes the v1 link-time text optimizer —
`representation_plan.rs`, `late_lower.rs`, the `native_*` lowering modules — which rewrote the
baseline lane's printed JavaScript. It was deleted in Phase 5 (W5h) once the D11 criterion held:
the jir lane reproduced all nine byte receipts within 2%, held every fixture in every mode, beat
the baseline `on` sizes on both FingerprintJS ports, and the 3-browser differential was clean.
The deletion also removed a known `js-opt=on` miscompile (phase-4 fixture 78). The text below is
kept as the specification the jir lane was verified against, and as the provenance of the receipt
numbers in `scripts/native-*-test.sh`; none of the machinery it names exists in the tree any more
(the report's `representation_plan` is now always `null`).

### Executable late IR and native representations (v1)

In `report` and `on`, each encodable monomorphic function or static carries a versioned owned CFG. The IR records stable crate and definition identities, canonical generic and type identities, typed locals and places, control flow, constants, aggregates, casts, calls, drops, assertions, ABI shape, allocator facts, layouts, statics, and canonical standard-library operation identities. A constant backed by anonymous immutable memory decodes into an owned typed value tree that mirrors the printed JavaScript value model: scalars, strings, aggregates and enum variants at their layout offsets, slices with their lengths, references into further immutable memory, and pointers to named globals. Mutable memory, vtable pointers, oversized or over-deep allocations, and unreadable bytes keep the item unavailable. Paths and source spans remain diagnostic metadata and cannot decide semantic duplicate equality. If one MIR operation cannot be encoded exactly, the complete item becomes unavailable with one deterministic reason.

Printed JavaScript remains beside late IR as the fallback. For each available item, a baseline shadow replays current MIR codegen and compares identity, JavaScript and source mappings, references, diagnostics, and linkage; it also verifies an exact owned-IR serialization round trip. A native replacement requires this `identity-gated` result. The executable lowerer is not a general replacement for current codegen: unsupported bodies continue to use their printed declaration.

The executable lanes are a native language kernel for supported ordinary scalar and aggregate CFG operations, semantic standard-operation bodies for native heap values, opaque move identities, and compiler-generated structural drop chains whose leaf drops already have nonthrowing native replacements. The language kernel can retarget supported direct calls and ordinary structs, tuples, arrays, references, slices, and statics without going through the baseline heap model.

At final link, the representation planner starts from baseline reachability, solves direct-call SCCs and nested value flow, joins export round trips, and selects one representation for every complete component. It tracks raw provenance, mutation, layout, allocator identity, pinned boundaries, unsupported calls and operations, and custom drop behavior. Value pairs that cross a direct call or drop boundary are directional: a pinned callee pins its callers transitively because the callee observes their values, while a pinned caller stays out of the callee's component, since the callee can serve its remaining callers through a private native clone while the pinned caller keeps calling the preserved baseline original. Only pin-free sides of a boundary join one shared component, and a staged region additionally verifies that every native callee slot receives a caller value planned with the same representation before any byte is committed. The native region stage closes over typed aggregate and direct-call dependencies, call SCCs, vtable slots, and otherwise unexplained incoming linker references. Representation-class anchors join exported round trips only, so unrelated internal values do not share one transaction merely because they use the same native class. Baseline-only fallback dependencies pin their exact targets.

An internal function with a native-only ABI can be emitted as a deterministic private clone while its original declaration remains available to baseline callers. Only prepared native callers are retargeted. Preserved originals force every incompatible target they call into the same clone fixed point, and a public native export cannot clone away from its original symbol. Baseline sidecars preserve an exact original body when a representation-neutral adapter must serve both carrier forms. The hypothetical reachability and size calculation includes live originals, clones, sidecars, and their dependencies. A transaction commits only if every member is proven and the complete reachable item graph becomes smaller; the linker then applies it atomically, recomputes references and reachability, runs DCE again, and reports only final reachable rewrites.

When complete executable late IR is unavailable, a separate fail-soft MIR scan can still record a narrow carrier certificate. The implemented certificate admits one exact direct `String` producer result whose only uses are one root shared borrow into the compiler-owned `String::as_str` adapter and one canonical compiler String drop. The linker may keep that caller byte-identical while joining it to the producer and both dual-carrier adapters. Any extra alias, projection, function-item use, cleanup use, clone, or identifier retarget declines the bridge.

Eligible `Vec<T>` values use `{buf, len, cap}` with initialized elements in `[0, len)`. Native slice views share the same buffer. The standard-operation lane covers construction with and without capacity, length and capacity reads, push, clear, slice views, emptiness, indexing, removal, owned-`Vec` extension, drop, and slice swap. It preserves explicit capacity, clears released slots, checks bounds and capacity overflow before mutation, moves owned elements out of an extended source, and calls exact monomorphic element drop glue in order when required.

An eligible immutable `String` is a host string. A mutable one uses `{text, cap, len}`: `cap` is Rust's capacity and `len` the exact UTF-8 byte length of `text`, both in bytes. No operation rescans the text to learn its length: `push`/`write_char` advance `len` by the scalar's width, `push_str`/`write_str`/`write_fmt` by the appended text's own measured length, `clear` and drop reset it, and every constructor measures its text once or knows it (empty, ASCII decimal, the baseline `len` leaf). `len()` is therefore a field read and building a string costs the bytes appended rather than the bytes already there (it was quadratic while every append measured the whole text). `cap` grows only to the new length when an append exceeds it, so it never falls below `len`. The text of either native string is always well formed and equal to what the baseline `Vec<u8>` would decode, because every piece that joins it is well formed on its own: each piece a host may supply — a `push_str`/`write_str` argument, each `&str` argument of a `format!`/`write!` site (taken where it is displayed, not after concatenation), a host string converted into a builder — passes through `__rt.native_str_wf`, which turns a lone surrogate (only a host can supply one) into the U+FFFD the baseline's bytes hold, and every other piece (a `char`, template literal text, integer/`bool`/`f64` text, a native `String` argument's text) is well formed by construction. Two adjacent pieces, in one site or across appends, therefore never pair into a scalar the baseline never saw, and the host is handed the same text from either representation. `RCGJS_SB_VERIFY=1` at link time routes every length read and every mutation through `__rt._sb_check`, which re-measures the text and aborts on drift. The current lane covers construction with and without capacity, UTF-8 byte length, capacity, `push`, `push_str`, `clear`, `as_str`, `fmt::Write::write_str`, `fmt::Write::write_char`, exact default `f64::to_string`, exact `ToString::to_string` for every fixed-width integer, `bool`, and `char`, default-Display `format!("{x}")` / `write_fmt` for those same types plus `&str`, `String`, and shortest `f64`, and drop. The compiler owns the same `String::new` / `with_capacity` constructor those sinks need, so a `format!`-only program does not have to keep `with_capacity` reachable. Exact compiler-marked `write_fmt` adapters and their String vtable keep baseline `fmt::Arguments`, `Option`, and `Result` carriers while accounting every native slot. The compiler-owned `as_str` adapter returns native text when it sees the exact disjoint native shape and otherwise calls a private copy of its baseline body. The compiler-owned String drop adapter returns for the exact mutable native shape and otherwise calls its exact baseline `Vec<u8>` drop target, which is pinned to the baseline body. Runtime helpers provide exact UTF-8 byte length, scalar append, character-boundary checks, UTF-8 byte to UTF-16 index conversion, and Rust-compatible default `f64` text.

Zero-sized or unsized vector elements, custom or unknown allocators, unsupported pointer widths, address or provenance observation, unsupported operations, indirect or foreign calls, user-defined destructors, missing exact drop glue, and pinned values keep the connected region on baseline. Capacity overflow is checked for the supported 32-bit target before allocation or mutation; the JavaScript target does not simulate host out-of-memory failure. Native regions also retain printed JavaScript when source maps are requested because the executable lowerers do not yet reproduce source-map entries.

When a committed native region reaches an export, the report records the exact export representation map and every emitted program or chunk carries `// rcgjs-artifact-representation-id: <id>`. The marker identifies one non-contractual aggregate ABI; it does not make the layout a JavaScript API or provide cross-artifact conversion.

## Reports, diagnostics, and cache keys

The version 6 JSON report accounts for every emitted byte as reachable items, link-generated items, or synthesized text. It lists the JavaScript syntax target and opaque raw-JavaScript validation policy; registered, requested, analyzed, emission-enabled, and actually applied passes; byte totals by crate and item; root chains; selected and rejected candidates; owned-IR availability; baseline-shadow status; planned and applied representations; artifact IDs; export maps; clones; and fallback reasons. A pass is `applied` only when a reachable emitted item contains its rewrite; selecting it in `on` appears separately under `emission_enabled`. An item whose body keeps one or more dispatch loops records a `structure` selection naming the form (`scoped-dispatch` or `whole-body-dispatch`), the region count, and whether the graph has a loop with more than one entry; a fully structured body records nothing. Stable sorting makes repeated equivalent builds deterministic. The backend and `jsc-build` reject optimization reports for chunked builds because per-chunk byte accounting is not implemented; reporting unsplit bytes beside published chunks would be misleading.

In `report` and `on`, each reachable rejected workspace source span produces at most one warning, even when monomorphization or duplicate object definitions repeat the miss. Dependency, `core`, and `alloc` misses stay in JSON. `js-opt-warnings=off` suppresses diagnostics without removing report data. Workspace membership is decided by the final linker using normalized lexical paths, so a `..` path cannot escape the configured root and still be classified as workspace source.

Rlibs merge optimization metadata for semantically identical duplicate definitions instead of discarding it. The object payload is format 20 (v1 wrote format 19; v2 added the binary member and the JIR-S body), the owned late-IR schema is version 11, the baseline-shadow schema is version 1, and the internal representation-plan schema is version 1. The report is version 6; since W5h its `representation_plan` section is always `null` (the key is retained so the report schema is stable), and the jir lane's decisions live in the `<out>.lane.json` sidecar instead. The `dedup` array lists each `{winner, merged, bytes_saved}` group the link-time duplicate merge committed. Stale or conflicting data is rejected with a clean-rebuild instruction.

Sysroots are immutable and keyed by the backend binary, rustc identity and target, patches, normalized emit-affecting options, and explicit Rust optimization level. Equivalent option order shares a key, while `off`, `report`, and `on` do not. This permits the mode matrix to coexist without overwriting one shared sysroot.

## Remaining analysis and fallbacks

Executable native regions are implemented, but several broader opportunities remain analysis-only or deliberately fall back:

| area | current state | required before broader emission |
| --- | --- | --- |
| General late-IR lowering | The native language kernel, semantic standard operations, opaque moves, and structural drop chains can emit executable replacements. Every other body keeps printed JavaScript. | Broader instruction, ABI, drop, and source-map coverage with the same identity and atomic-region gates. |
| Native Promise continuations | A post-coroutine-transform eligibility analysis and native continuation ceiling benchmark are report-only. The analysis accepts canonical awaits of concrete coroutines and of exact compiler-owned awaitable types (the runtime crates' promise-await and task-yield futures and `core::future::ready::Ready`), and records which of those a graph awaits beside its coroutine, await-loop, and suspension counts. The owner-aware terminal stays pinned. | Exact continuation lowering plus at least a 5% stable win on a representative workload. |
| Iterator, bounds, and broader formatting | The kernel covers narrow proven scalar reference, assertion, division, and unreachable control-flow cases. Exact non-escaping `Enumerate<Take\|Skip<SliceIter<u8>>>` loops may become checked cursors. Exact String `fmt::Write` adapters, default `f64::to_string`, primitive `ToString::to_string`, and default-Display `format!("{x}")` / `write_fmt` for integers, `bool`, `char`, `&str`, `String`, and shortest `f64` can emit. General iterators, other element types and adapter orders, bounds, user `Display`, Debug, and width/fill/precision stay on baseline `core::fmt`. | Exact lowering and focused benchmark evidence before broader cases become applied. |
| Wider native heap API | The listed `Vec`, `String`, slice, and structural-drop paths can emit. Every unsupported operation or connected caller keeps its complete region on baseline. | Differential semantics for each added operation, including capacity, panic, alias, move, and drop behavior. |
| Parameterized duplicate merge | Only bodies that are the same code merge. Bodies that agree except in which item they name keep one copy each. | A win worth the indirection. On the FingerprintJS program after the exact merge, 43 such groups covering 216 items are left, and passing the differing names as arguments would save 7,213 raw bytes at best, against a wrapper on every call into a group. |

The formatting analysis names the consumer of every eligible `Arguments` value, so `native-default-display-builder-format-bbN` and `native-default-display-builder-write-fmt-bbN` separate the two owning sinks. Proven default-Display templates now emit: static pieces plus already-lowered argument texts are concatenated into the same native `String` layout `String::new` / `with_capacity` already lower (a host string, or `{text, cap, len}`). The compiler owns that constructor, so a `format!`-only program does not need a reachable `String::with_capacity` item. User `Display`, Debug, and width/fill/precision beyond primitive `to_string` stay on baseline `core::fmt`, including `f32` and any `f64` site that is not default-shortest.

The kernel inlines a format site only on exact identity, never on shape. The consumer must be the linked `alloc::fmt::format` item or the exact `String::write_fmt` operation, and the elided constructors must be the linked `Arguments::new` / `Argument::new_display` items — the identities travel from the callee bodies' standard semantics, so a user function that takes one `fmt::Arguments` argument is never rewritten. The pipeline elides only the constructor apparatus: a producer call feeding a template (`value.to_string()`) keeps its emission, and a `String` display argument needs a planned native representation, not a runtime type test. A format-machinery call the pipeline cannot prove declines the whole item rather than emitting generically, because a staged native constructor and a baseline consumer disagree on the `Arguments` encoding. For the same reason every vtable-referenced item keeps its baseline representation: the dispatcher passes carriers no collected body shows. `examples/core-tests/57_native_format_sinks` pins all of these shapes differentially across the three modes.

### Size-unlock receipts

Three fixtures and two FingerprintJS rebuilds measure the remaining named holes. They do not change optimizer defaults.

| Receipt | Command / fixture | What a win looks like |
| --- | --- | --- |
| `&str` index does not pull `fmt` | `examples/core-tests/54_str_index_no_fmt` (`.absent` lists `fmt::write`, `escape_debug`, `unicode_data`, `flt2dec`, `pad_integral`) | Patterns stay absent after a sysroot rebuild that includes the `slice_error_fail_rt` stub. |
| Host string sink unpins native `String` | `examples/core-tests/55_native_host_string` | `js-opt=on` commits a native `String` region that is passed to `js_bindings!` as `&str`; `off`/`report` stay identical; mixing into `JsValue` still records `HostBoundary`. |
| `format!("{x}")` does not pull the formatter | `examples/core-tests/56_native_format_args` via `scripts/native-format-args-test.sh` | `js-opt=on` emit for `u32` / `&str` / default `f64` contains no `flt2dec` or `pad_integral`, and the string matches `off`. |
| Format shapes the lowering must not claim | `examples/core-tests/57_native_format_sinks` via `scripts/native-format-sinks-test.sh` | A `to_string` producer feeding a template, a user `fmt::Arguments` façade, and one- and two-site `write!` sinks behave identically in `off`, `report`, and `on`. |
| Compact FingerprintJS | `experiments/fingerprintjs-rust-compact` `node tools/build.mjs --optimizer on --rust-opt-level z --mir-inline on --scheduler boundary` | 2026-08-14 rebuild after the `slice_error_fail_rt` stub: min ESM 159,764 B (from 192,850), gzip-9 43,910 B (from 53,594), raw program 521,642 B (from 614,026). `fmt::write` / `escape_debug` / `flt2dec` / `pad_integral` are gone. |
| Idiomatic FingerprintJS `on` | `experiments/fingerprintjs-rust` `node tools/build.mjs --optimizer on --scheduler boundary` | 2026-08-14 rebuild after all three slices: min ESM 258,721 B (from 277,937), gzip-9 64,493 B (from 70,401), raw program 898,738 B (from 947,810). `print_debug` still reaches `flt2dec`. |

Planned representations remain analysis metadata. `planned_clones` lists planner candidates. `optimized_clones` lists only committed variants as `{source, emitted, scc}`; `source == emitted` means an in-place replacement, while a distinct emitted name is a private native clone reached only from prepared callers. Baseline sidecars are not native clones and are not reported as optimized. A declined transaction emits none of its variants or sidecars and never mixes native and baseline representations.

## Measurements

**These are v1's numbers, and they are the BASELINE lane's.** Every figure in this section was
measured in the v1 checkout on 2026-08-14 at object format 19, and none of it has been re-measured
in v2: the object payload is format 20 here, and the whole section describes the frozen text
optimizer, which no longer exists (see the "History" heading above). It is kept because the jir
lane's receipts were defined against these numbers, and a receipt needs its provenance. For v2's
own measurements — the jir lane with `js-opt=on`, the FingerprintJS
ports, the Murmur gate — read `checkpoints/phase-3.md`, which supersedes the FingerprintJS numbers
below.

The Store, FingerprintJS package, and protected Topcoat checks below were rebuilt on 2026-08-14 with object format 19, late-IR schema 11, report version 5, baseline-shadow schema 1, internal representation-plan schema 1, and the historical v1 report representation section schema 3. That historical JIR-lane sidecar was schema 8; the current sidecar is schema 15. The FingerprintJS browser performance section below remains the frozen 2026-08-12 object-format-12 comparison; the current clean-build/browser/runtime record is `checkpoints/north-star-baseline.json`, with later tranches recorded in the phase checkpoints. These results are evidence for implemented compiler slices, not a release-default decision.

### Pure Rust store

The current Store checkpoint uses Rust opt level 0, minified compiler output, a minimal runtime, one fresh-process round per mode, five warmups, and thirty measured iterations. It is a semantic and size smoke test, not a stable runtime benchmark.

| mode | total raw | gzip -9 | Brotli 11 | p50 smoke | p95 smoke |
| --- | ---: | ---: | ---: | ---: | ---: |
| `off` | 110,523 B | 24,881 B | 20,795 B | baseline | baseline |
| `report` | 110,523 B | 24,881 B | 20,795 B | not timed | not timed |
| `on` | 13,783 B | 4,312 B | 3,785 B | -56.58% | -52.53% |

`off` and `report` are byte-identical: each has a 90,517 B program and a 20,006 B minimal runtime. `on` has an 11,020 B program and a 2,763 B minimal runtime. The combined raw, gzip, and Brotli reductions are 87.53%, 82.67%, and 81.80%. All modes produce the exact state parity hash `0806e4849b66cf93cc7f4185e1f4c69acf9afefe03e12484cc3dfec209c8fdc0`.

The one-round runtime samples were 5.367 ms p50 and 8.784 ms p95 in `off`, then 2.331 ms p50 and 4.170 ms p95 in `on`. One round cannot estimate between-process variance, so these point changes are not accepted performance evidence. No Store runtime improvement is quoted for promotion until a stable multi-round rerun reproduces it.

The current Store report commits 32 reachable native rewrites: 15 ordinary language-kernel bodies, 16 semantic standard-operation bodies, and one compiler structural-drop body. All 13 exports use the same native `Store.rows: Vec<Row>` and `Row.label: String` map. The emitted program carries `// rcgjs-artifact-representation-id: rcgjs-repr-v1-d58e2dac9e5da3504b0b18bdc4cb50c7`, so the store can round-trip only through this artifact's exports.

### Pass attribution and Rust optimization levels

The developer harness builds optimizer `off`, `report`, no pass, each singleton pass, cumulative prefixes, each leave-one-out set, and `off` plus all passes at Rust optimization levels `0`, `1`, `2`, `s`, and `z`. Its compact [attribution record](bench/optimizer-matrix/results/current.json) includes compiler, sysroot, pass, artifact, report, and raw-result hashes.

The existing full attribution record predates executable native Store regions, so its native-representation conclusions are not current promotion evidence. The cumulative Store result above must still be repeated independently, cumulatively, and with each candidate removed before any pass is promoted. The global Rust optimization level remains `0` until a fresh stable matrix and the protected browser suites clear the normal gates.

### Async poller microbenchmark

The async benchmark runs 1,000 operations per sample, five warmups, twenty samples in each of ten isolated-process rounds, and checks result parity before measuring.

| workload | mode | total raw | gzip -9 | Brotli 11 | p50 | p95 |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| ready Future | `off` | 40,039 B | 8,475 B | 7,326 B | 1.342 ms | 1.628 ms |
| ready Future | `future-poller` | 26,515 B | 7,390 B | 6,462 B | 0.266 ms | 0.387 ms |
| once-suspended Future | `off` | same artifact | same artifact | same artifact | 3.667 ms | 4.247 ms |
| once-suspended Future | `future-poller` | same artifact | same artifact | same artifact | 2.195 ms | 2.679 ms |

The ready workload improved by 80.22% p50 and 76.37% p95. The once-suspended workload improved by 39.60% p50 and 40.23% p95. The poller-only artifact fell by 33.78% raw, 12.80% gzip, and 11.79% Brotli. `off` and `report` program and minimal-shim hashes were identical.

### FingerprintJS optimizer matrix

Superseded for v2 by `checkpoints/phase-3.md`, which measures the same ports on the jir lane with
`js-opt=on`: idiomatic 260,671 B minified / 64,049 gzip / 52,245 brotli, compact 163,443 / 43,225 /
32,875. The paragraphs below are the v1 baseline-lane record and are not a v2 result.

The v1 object-format-19 `on` boundary package builds, imports in ESM and CJS SSR contexts, and passes its TypeScript consumer check. Its raw compiler program is 1,031,528 B, readable ESM is 991,544 B, and identically Terser-minified ESM is 306,304 B. The report commits 19 language-kernel rewrites plus one in-place opaque move identity, plans 28 native variants, commits that one identity variant, and has no native exports. The exact carrier certificate for `prepare_natural_fonts` validates and reaches transaction sizing, but the connected `natural_font_text` String region remains baseline because it does not shrink the complete reachable item graph. Exact `f64::to_string` is also recognized and planned but remains in a declined String transaction. Two alternating captures produced zero normalized differences in Chrome 151.0.7922.138, Firefox 153.0.4, and Safari 26.6, with both monitoring arrays empty; the [current parity records](../../experiments/fingerprintjs-rust/tests/results/native-pivot-final-current/) identify candidate SHA-256 `253e6345ca41a59fe5fa3d4babb4aec7129208b10b360b02c5885b83afccfbdd`. This checkpoint did not run performance tests; the package checker's six known bundle-size gates remain failed as allowed.

The v1 minified IIFE was 306,301 B against a 36,746 B gate, a 269,555 B gap. A read-only reachability counterfactual that replaces the entire shared `f64` Display entry with an empty body removes 97 items and 45,901 B from the minified output, but still leaves about 260,400 B, or 7.1 times the gate. That is an optimistic ceiling rather than an implementable rewrite: exact Display also includes fixed precision, padding, flags, formatter errors, and arbitrary rounding behavior. The best bounded future formatting seam, default-shortest `f64`, accounts for at most 10,442 minified bytes before replacement cost. Encoding indirect or anonymous constants would make more late IR available but would commit no current planned FingerprintJS transaction. No broader formatter or constant-codec slice is therefore justified solely by this size target.

Root chains in the current `on` boundary report split the 200,493 B formatting cluster across three unrelated gateways. `api::print_debug` holds 125,318 B in 54 items, all of it reached through `Argument::new_display` and the `f64` Display entry into flt2dec. `str::slice_error_fail` holds 35,382 B in 31 items: the panic message inside `str::contains` is what keeps `usize` Display, `char` Debug, the escape iterators, and the Unicode case tables alive, and no formatting seam reaches it. The two `format_args!` templates in `sources_runtime` hold 36,628 B in 24 items and are the only part a template lowering would free. The report lists 6 eligible template sites, all consumed by the owning `fn(Arguments) -> String`, and 1 declined float argument. Exact primitive `ToString::to_string` has no site in this program at all: `to_string` is instantiated only for `&str` and `f64` here, and the reachable integer formatting arrives through templates and Debug instead. The one `to_string` instance whose result the planner does select as a native `String` is still declined by the region size gate, which remains the same blocker recorded above.

The 2026-08-14 optimization round rebuilt this workload after the `link-dedup` pass, the primitive `to_string` lanes, the `sroa` pass with the opt-in app-crate MIR inlining, the scoped-dispatch coroutine structurizing, and the decline-instrumented severing planner landed. The default `on` boundary package is now a 947,810 B raw program and a 277,937 B minified ESM at 70,401 B gzip-9 and 57,422 B Brotli-11, against 306,304 B minified before the round. The dedup section records 77 groups over 268 merged items, the five coroutine dispatch whales fall from 337 states to 37, and the planner plans 36 variants with a recorded decline reason on every one that does not commit. Both `off` and `on` builds produced zero normalized differences in Chrome 151.0.7922.140, Firefox 153.0.4, and Safari 26.6, and the full three-browser minified benchmark improved every phase metric over the pre-round record, with the total p95 geometric mean falling from 15.9 percent over upstream to 13.9 percent. The six bundle-size gates and the 5 percent runtime gate remain failed, so the release default stays `off`; the [round benchmark records](../../experiments/fingerprintjs-rust/bench/results/round-bench/) hold the raw data, with the `boundary` slot holding the `off` build and the `fiber` slot holding the `on` build.

Focused String evidence is much smaller because every connected operation is supported. The native String fixture falls from 27,098 B to 6,508 B, and the `f64`-to-String fixture falls from 331,511 B to 3,646 B. A mixed-carrier fixture falls from 20,794 B to 4,023 B while keeping its baseline caller on the original `pair_total` symbol and retargeting only the prepared caller to a private `$native` clone. The exact default-float helper matches Rust on 1,048,615 directed and deterministic bit-pattern cases, with result SHA-256 `e2cab7093ec67a9f6f4879ff0eb32e0ce8eac98798d27e8a89a1ae7d71029f88`.

The frozen object-format-12 FingerprintJS boundary workload below was rebuilt and checked in all three modes. `off` and `report` produced byte-identical compiler programs, shims, generated facades, declarations, and packaged formats. Package API and format checks, SSR-safe ESM and CJS imports, and TypeScript contract checks passed in every mode; the package checker's only reported failures are the experiment's already-failed bundle-size gates. Differential runs for `off` and `on` produced zero normalized differences in Chrome, Firefox, and Safari.

| artifact | `off` | `on` | change |
| --- | ---: | ---: | ---: |
| Raw compiler program | 1,243,691 B | 1,031,767 B | -17.04% |
| Identically Terser-minified ESM | 339,006 B | 305,036 B | -10.02% |
| Identical-Terser ESM gzip-9 | 75,415 B | 73,374 B | -2.71% |
| Identical-Terser ESM Brotli-11 | 60,383 B | 59,030 B | -2.24% |
| Shipped minified ESM, including notice | 340,329 B | 306,359 B | -9.98% |

The direct `on`-versus-`off` browser comparison used the p95 geometric mean across Chrome, Firefox, Safari, and the idle, CPU, and DOM scenarios. Negative values are lower and therefore better for latency and delay. Workload progress is the exception: higher is better, so its negative change is a regression.

| p95 geometric mean | `on` versus `off` |
| --- | ---: |
| Total | -0.30% |
| Module evaluation | -2.61% |
| Immediate `get` | +0.72% |
| Delayed `get` | -2.98% |
| `load` | +2.08% |
| Lazy hash | +0.48% |
| Repeated `get` | -7.41% |
| Event-loop delay | +1.07% |
| Input delay | -0.08% |
| Maximum rAF gap | +7.77% |
| Workload progress | +0.84% |

The 810-record aggregate result is mixed: repeated `get` and total latency improved, but maximum rAF gap exceeded the aggregate 5% limit and 17 of 234 protected browser and scenario cells exceeded the 5% regression limit. Ten improvements of at least 5% had a bootstrap interval below parity. No browser recorded an error or a new long task. The default-enablement gate therefore failed.

The frozen evidence is the [direct performance comparison](../../experiments/fingerprintjs-rust/bench/results/optimizer-minified-final/on-vs-off.json), its [Chrome](../../experiments/fingerprintjs-rust/bench/results/optimizer-minified-final/chrome.json), [Firefox](../../experiments/fingerprintjs-rust/bench/results/optimizer-minified-final/firefox.json), and [Safari](../../experiments/fingerprintjs-rust/bench/results/optimizer-minified-final/safari.json) records, the [off](../../experiments/fingerprintjs-rust/bench/results/optimizer-esm-off.json), [report](../../experiments/fingerprintjs-rust/bench/results/optimizer-esm-report.json), and [on](../../experiments/fingerprintjs-rust/bench/results/optimizer-esm-on.json) ESM measurements, the [off](../../experiments/fingerprintjs-rust/bench/results/optimizer-reports/off-boundary.json), [report](../../experiments/fingerprintjs-rust/bench/results/optimizer-reports/report-boundary.json), and [on](../../experiments/fingerprintjs-rust/bench/results/optimizer-reports/on-boundary.json) compiler reports, and the [final object-format-12 browser parity results](../../experiments/fingerprintjs-rust/tests/results/final-object12/).

### Protected Topcoat matrix

| check | result |
| --- | --- |
| Root workspace | `cargo test --workspace --all-features` passed. |
| Compiler workspace | Passed; the backend has 367 tests. |
| Async, each of `off`, `report`, and `on` | 7/7 fixtures and 44/44 behavior checks passed. |
| DOM and hydration, each mode | 25/25 passed with 441 runtime-call records and exact expected traces. |
| Current optimized rerun | `on` passes async 7/7 with 44 behavior checks and DOM 25/25 with 441 runtime-call records against the final backend. |
| Prior all-mode size evidence | `off` and `report` normalize exactly after removing fresh procedure UUIDs; the readable DOM aggregate was 224,888 B in both modes, and `on` preserved trace hash `f1dac868f29c121906bd02441233964ad1ba83a9c313fe444add2922c9b28e96` at 194,599 B. |
| Prior readable async aggregate | 332,207 B in `off`, 191,984 B in `on`, a 42.2% raw reduction. |

These results protect the existing Topcoat paths, but they do not cover every release gate across browser runtime, responsiveness, heap, gzip, and Brotli. The optimizer therefore remains off by default.

## Reproduction

Run the compiler checks from `spike/rustc-codegen-js`:

```sh
cargo test --workspace
./scripts/optimizer-diagnostics-test.sh
./scripts/sysroot-cache.test.sh
node --test scripts/minimal-shim.test.mjs
node --test scripts/host-future-driver.test.mjs
node --test scripts/native-values.test.mjs
./scripts/native-kernel-contract.sh
./scripts/native-f64-differential.sh
./scripts/native-int-differential.sh
```

Build the required keyed sysroots, then run the protected mode matrix:

```sh
./scripts/build_sysroot.sh
for mode in report on; do
  JS_EXTRA_ARGS="js-opt=$mode" JSC_SYSROOT_ARGS="js-opt=$mode" ./scripts/build_sysroot.sh
done

./scripts/representation-test.sh
./scripts/native-clone-test.sh
./scripts/native-vec-test.sh
./scripts/native-string-test.sh
./scripts/native-host-string-test.sh
./scripts/native-capacity-test.sh
./scripts/native-vec-drop-test.sh
./scripts/native-vec-index-test.sh
./scripts/native-vec-remove-test.sh
./scripts/native-vec-swap-test.sh
./scripts/native-structural-test.sh
./scripts/native-vec-extend-test.sh
./scripts/native-f64-string-test.sh
./scripts/native-int-string-test.sh
./scripts/native-format-args-test.sh
./scripts/formatting-test.sh
./scripts/dedup-test.sh

for mode in off report on; do
  ASYNC_OPT_MODE="$mode" ./scripts/async-test.sh
  DOM_OPT_MODE="$mode" ./scripts/dom-test.sh
  EMIT_OPT_MODE="$mode" ./scripts/emit-test.sh
  MODULE_OPT_MODE="$mode" ./scripts/module-test.sh
  DTS_OPT_MODE="$mode" ./scripts/dts-test.sh
  EXTERN_OPT_MODE="$mode" ./scripts/extern-test.sh
done
```

The `native-*` scripts above run on the jir lane by default since W5h; `NATIVE_LANE=baseline`
instead checks that the baseline lane's `js-opt=on` output is byte-frozen to `off`.

Each of those six suites pins its own compile flags, so it takes the mode as its OWN environment
variable rather than through `$JS_EXTRA_ARGS` — a golden suite that inherited the caller's flags
would not have goldens. Passing `js-opt=` in `$JS_EXTRA_ARGS` to one of them does nothing; it is
dropped. Add `JS_EXTRA_ARGS='js-lane=jir'` to run them on the JIR lane. `scripts/matrix.sh`'s
`lane-suites-opt` mode runs the four text suites this way in one command.

Run the two focused benchmarks:

```sh
./bench/melange/optimizer-build.sh
node ./bench/melange/optimizer-compare.mjs

for mode in off report on; do
  JS_EXTRA_ARGS="js-opt=$mode" JSC_SYSROOT_ARGS="js-opt=$mode" ./scripts/build_sysroot.sh
done
./bench/async-poller/build.sh
node ./bench/async-poller/run.mjs
```

Raw benchmark data is written to `bench/melange/out/optimizer/results.json` and `bench/async-poller/out/results.json`. Per-mode compiler reports are under `bench/melange/out/optimizer/reports/`.

The developer attribution harness builds `off`, `report`, no-pass, every singleton pass, the cumulative prefixes, the cumulative set with each pass removed, and Rust optimization levels `0`, `1`, `2`, `s`, and `z`. It records compiler, sysroot, artifact, input, and report hashes before timing:

```sh
./bench/optimizer-matrix/build.sh --build-sysroots
node ./bench/optimizer-matrix/run.mjs
node ./bench/optimizer-matrix/ceilings.mjs
node ./bench/optimizer-matrix/check.mjs
node ./bench/optimizer-matrix/summarize.mjs
```

## Default decision

`OptimizerMode::Off` remains the release default. `report` is the safe way to collect the next cross-crate evidence, and `on` is suitable for parity and performance experiments. A pass should become default only after it has an exact semantic matrix, a statistically credible improvement of at least 5% on a representative primary metric, no protected regression above 5%, and no new 50 ms task.
