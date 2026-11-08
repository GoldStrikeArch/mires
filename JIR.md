> Recovery baseline: this document was imported from the September 7 v2 snapshot.
> It describes historical mechanisms and limits, not completed October recovery or production qualification.
> See `docs/V2-RESTORATION.md` and the current `VERDICT.md`.

# JIR identity: what makes two bodies the same body

One monomorphic Rust item can reach the link step several times: two crates codegen it, two codegen
units of one crate codegen it, or the text optimizer proves two differently named items print the
same JavaScript and merges them. Every one of those cases needs an answer to "which body survives",
and v1's answer was to give up on the item's late IR whenever the bodies were not byte-equal.

This document is the v2 rule, the normal form it compares, and the autopsy of v1's 74 duplicate
conflicts on the FingerprintJS idiomatic port.

## What v1 did

`late_ir.rs::merge` (v1, lines 281–360) combined two `LateIrAnalysisSummary`s conservatively: any
difference between two `Availability::Available(body)` values became
`Availability::Unavailable(DUPLICATE_CONFLICT)`, and the conflict was sticky — once set, every later
merge kept it. `item.rs::merge_duplicate_metadata` turned that into an optimizer miss
(`late-ir-analysis`), and `link.rs::dedup` (v1, line 702) merged metadata for items whose printed
text matched and raised a hard error when it did not.

Two facts made that expensive. Body equality was **byte equality of the encoded body**, which
includes `StorageLive`/`StorageDead`/`Nop` statements, `FalseEdge`/`FalseUnwind` terminators, MIR
statement indices, block numbering and intern ids — none of which change what the body does. And a
difference cost the item its *availability*, not just one of its two bodies, so a duplicate could
take an item out of the JIR lane entirely.

## The v2 rule

Two relations, two rules. They are separate because the proofs behind them are separate.

### 1. Duplicates — the same item, twice

The identity is `SemanticKey = (link_name, Option<ItemId>)`: the emitted JavaScript name, which is
what v1 keyed on, together with the stable Rust identity behind it (`DefPathHash` plus a stable hash
of the instantiation). `jir::summary::merge_duplicates(a, b)` decides:

| situation | decision |
| --- | --- |
| the objects' headers disagree (`merge_headers`: JIR schema, rustc version, data layout, emit options) | `Error` — a hard link error naming the rebuild that fixes it |
| different `SemanticKey`, same `link_name` | `Error(NameCollision)` — v1's "two different definitions reached the same program", and the only case that is still an error |
| same key, equal normalized `semantic_hash` | `Merge` — keep either body, union the origins |
| same key, a body on one side only | `Available(side)` — keep the side that has one; not a decline |
| same key, different normalized hashes | `Choose { keep, chosen, rejected }` — keep the **smaller hash**, record `Decline::Merge(BodyChoice { chosen, rejected })` |

Text equality no longer participates, and a body difference is never unavailability. The min-hash
pick makes the outcome independent of the order the linker read the objects in.

### 2. Aliases — two items, one printed text

The text optimizer's `link_dedup` pass merges items whose printed JavaScript is byte-identical under
*different* names — two monomorphizations of one generic that erase to the same code — and points
every reference at one winner. Those items are **not** the same `SemanticKey`: their type arguments
differ, so their bodies differ, legitimately. The proof that the merge is sound is the printed text,
which the pass has already established.

`jir::summary::merge_aliases(a, b)` is therefore `merge_duplicates` without the identity check:
never an error, never a loss of availability, and the same deterministic min-hash body pick recorded
as `Decline::Merge(BodyChoice)`. This is the rule that removes all 74 of v1's conflicts; see the
autopsy below.

## The normal form

`jir::summary::NormalizedBody::of(body, interns)` rewrites a body into the one form two objects have
to agree on, and `semantic_hash(body, interns)` is its digest (`Hash128`, SipHash-1-3-128 with fixed
keys — never `DefaultHasher`, because this is persisted). The rewrite:

1. **Storage markers and nops go.** `StatementKind::{StorageLive, StorageDead, Nop}` are dropped:
   liveness annotations for a stack the JavaScript lowering does not have, and holes a MIR pass left.
2. **Borrowck edges go.** `Terminator::FalseEdge { real, .. }` and
   `Terminator::FalseUnwind { real, .. }` become `Goto(real)`. Neither the imaginary edge nor the
   false unwind edge is taken at run time.
3. **Blocks are renumbered in reverse postorder** from the entry block, over the successor relation
   *after* (2). Blocks the walk does not reach are dropped, so a pass that made a block unreachable
   and a pass that also deleted it agree.
4. **Locals are renumbered**: the return local and the arguments first, in declaration order (their
   positions are the JavaScript ABI), then every other local in order of first mention in the walk of
   (3). A local no surviving statement mentions is dropped.
5. **Origins take no part.** They are not in `Body` at all. The one source position a body carries
   semantically, `Call::track_caller`, is kept: it is spelled into a `Location` value.
6. **Interned strings, byte blobs and source facts are re-interned by content**, in first-mention
   order, and hashed as content. Ids assigned by two different codegen units therefore compare equal.
7. **Types are hashed as they are.** A `TypeId` is already a global stable hash.

The result is a `Body` again, which is what lets `examples/jir-conflicts.rs` walk two normal forms in
parallel and name the first field that differs.

`ItemSummary` now carries both hashes, and the difference between them is the point:

* `body_hash` — the content hash of the body's postcard bytes. It addresses the blob in section S3
  and is the check the reader makes after inflating it. A hash of bytes, not of meaning.
* `semantic_hash` — the hash of the normal form. The identity the two merge rules compare, available
  from the eagerly read summary index so dedup never has to inflate a body.

Adding the field is a positional change to a postcard struct, so `JIR_SCHEMA` moved from 1 to 2; an
object written under schema 1 is rejected with a rebuild diagnostic rather than misread.

## The 74-conflict autopsy

v1's `js-opt=on` FingerprintJS boundary build
(`experiments/fingerprintjs-rust/bench/results/optimizer-reports/on-boundary.json`) reports 1,188
reachable items, 83 of them late-IR unavailable: 9 synthesized and **74 with the reason "identical
JavaScript definitions carried conflicting late-IR bodies"**.

The plan's verified-findings item 5 guessed those 74 were MIR-opt drift between the sysroot (built
with `-Zinline-mir`) and the application crate (built without it). **That is not what they are.**

The same report's `dedup` array — the text optimizer's identical-JavaScript merges — has exactly 74
groups with exactly 74 distinct winners, and the two sets are the same set:

```
dedup groups                     74
distinct winners                 74
conflicts                        74
conflicts that are dedup winners 74
winners that are not conflicts    0
conflicts that are not winners    0
```

So every one of v1's duplicate conflicts is a **link-dedup winner**, not a cross-crate duplicate. The
merged siblings are different monomorphizations — `core$ptr$drop_glue$h0133…` absorbing thirteen
other `drop_glue` instantiations, `core$array$iter$impl_7_drop$…` absorbing fourteen, eleven
`core$pin$impl_6_new_unchecked$…`, and so on — whose printed JavaScript is byte-identical and whose
late-IR bodies are legitimately different, because their type arguments are. v1 merged their
summaries with the same conservative `merge` it used for real duplicates, saw two different
`Available` bodies, and declined the winner.

Three consequences:

1. **Normalization alone would not have fixed them.** These bodies differ in their `TypeId`s, which
   normalization deliberately keeps. The fix is the *rule*, not the normal form: an alias merge picks
   a body (`merge_aliases`) instead of declining availability.
2. **The mixed-MIR-pipeline hypothesis is not what happened.** It is also not what happens to the
   duplicates: the numbers below show 69 real cross-crate duplicates whose bytes differ and whose
   normal forms do not, on a program whose sysroot *is* built with `-Zinline-mir` and whose
   application crate is not. The autopsy tool classifies genuine normal-form differences as
   `inlined` / `shape` / `content`; nothing in this program reaches that verdict.
3. **Normalization is still load-bearing**, just for the other relation: it is what lets a real
   cross-crate duplicate merge without a decline, and what keeps the choice deterministic when it
   cannot.

### The tool

```
cargo run -p rcgjs-object --example jir-conflicts -- \
  [--limit N] [--expect <names>] [--quiet] <file.rlib|object>...
```

It reads every format-20 object out of the given rlibs and bare objects, checks the headers agree,
groups every item by JavaScript name across objects, and for each group of two or more prints one of
five verdicts: `identical` (same postcard bytes), `normalized-identical` (bytes differ, normal forms
do not — v1 declined these, v2 merges them), `different` (normal forms differ; the tool names the
first differing field and classifies the cause as `inlined`, `shape` or `content`), `name-collision`
(one name over two Rust items — still a hard error) and `no-body`. `--expect` takes a list of names —
v1's 74, for instance — and reports what v2 makes of each.

### Numbers on the v2 FingerprintJS boundary build

`cargo run -p rcgjs-object --example jir-conflicts` over the whole `js-opt=report` boundary program
— the keyed sysroot's `libcore`, `liballoc` and `libcompiler_builtins`, `libtopcoat_js.rlib`, the
application rlib and both program objects:

```
objects                        7
items                       5194
header-mismatch                0
duplicate-names              122
verdict.identical             53
verdict.normalized-identical  69
verdict.different              0
verdict.name-collision         0
verdict.no-body                0
```

122 JavaScript names are carried by more than one object. 53 of them have byte-identical bodies
everywhere. The other **69 have different postcard bytes and one normal form** — v1 would have
declined every one of them, and v2 merges every one of them. Nothing in this program is genuinely
different, and no name is carried by two Rust items.

A representative group, one item codegenned by three crates:

```
alloc$alloc$impl_0_alloc_impl_runtime$h4a47ccc633324138
  blob 911a7756…  normalized 57c130ed…  liballoc[…cgu.0.rcgu.o]  :: alloc::Global::alloc_impl_runtime
  blob b90a2568…  normalized 57c130ed…  libtopcoat_js[…cgu.0.rcgu.o] :: alloc::alloc::Global::alloc_impl_runtime
  blob 7dd10d7b…  normalized 57c130ed…  libfingerprintjs…[…cgu.0.rcgu.o] :: alloc::alloc::Global::alloc_impl_runtime
```

Three blob hashes, one normalized hash — and three different `item_path`s, which is the origin table
being outside equality exactly as the schema says.

Run against v1's list of 74 conflicting names (`--expect`):

```
expected.single                68
expected.identical              5
expected.normalized-identical   1
```

Not one of the 74 lands in `different` or `name-collision`. 68 of them appear in the program
**exactly once**, so no duplicate rule of any kind can have produced v1's conflict for them; the
remaining 6 are duplicates whose bodies agree. That is the same conclusion as the `dedup`
cross-check above, reached from the objects instead of from the report.

Note also what the numbers say about the mixed-MIR-pipeline hypothesis for *duplicates*: the sysroot
in this build is compiled with `-Zinline-mir` and the application crate is not, and all 69 differing
pairs still share a normal form. The drift that exists between the two pipelines, in this program, is
exactly the storage-marker, block-numbering and intern-id kind the normal form erases.

## What is integrated

Part (b) wired the rule into the link step. Every call site below is live.

### `jsc-link/src/link.rs`

* **`Read::extend`** runs [`merge_headers`] once per object as it is read, against the first format
  20 header seen, and reports a mismatch as an error naming both objects and the rebuild that fixes
  it. Once per *object pair*, not once per item: the answer is a property of the pair. A format 19
  object carries no header and is held to nothing.
* **`dedup`** no longer compares printed text. It compares `SemanticKey`s and normalized hashes
  through `merge_duplicates`, and acts on the decision: `Merge` unions the metadata,
  `Available(side)`/`Choose { keep }` adopt the winning side's body **and its text** through
  `adopt_body`, and only `MergeError` reaches the old "two different definitions" error. `adopt_body`
  deliberately moves the body, text, refs, map and zombies but *not* the linkage: objects are read
  before rlibs and an rlib's items are demoted as they are read, so adopting a whole `LinkItem` from
  an rlib would quietly un-root the program's entry point.
* **`write_lane_report`** writes `-Cllvm-args=js-lane-report=<path>` from the table after dedup and
  reachability, one row per reachable item, `bytes` being the printed item's own length. An item
  with a decoded body is `semantic`, not `available`: Phase 1 still prints from the baseline lane.
* **`resolve`** collects `ConflictRecord`s from both merge sites and writes them to
  `-Cllvm-args=js-jir-dump-conflicts=<path>`.

### `jsc-link/src/link_dedup.rs`

`commit` calls `choose_alias_body` for every duplicate it absorbs, which is `merge_aliases` plus the
autopsy record. Their bodies are already decoded at that point — this pass runs after the reachable
ones were — so nothing is inflated for it.

### `rcgjs-object/src/item.rs`

`merge_duplicate_metadata` unions the metadata and nothing else. The two `OptimizationMiss`
records — `late-ir-analysis` and `baseline-shadow` — are gone, and the two summaries now merge
through `merge_choosing` rather than `merge`.

`LateIrAnalysisSummary::merge_choosing` (`late_ir.rs`) and `BaselineShadowSummary::merge_choosing`
(`baseline_shadow.rs`) are the v2 rule for the frozen schema-11 summaries the v1 text optimizer
reads: run v1's merge for its diagnostics, carrier bridges and allocator provider, and then, when
the only thing that conflicted was the body, replace the `DUPLICATE_CONFLICT` degrade with the
smaller of the two availabilities *by encoded bytes* — smaller by bytes rather than "the first one
read", so the outcome does not depend on archive order. A **version** mismatch keeps v1's degrade:
two schema versions in one program is a stale build, which the reader's `validate` already reports.

That is the change that takes the FingerprintJS report to zero conflict-caused unavailability.
Origins are unioned in the only sense they can be: an origin table is positional, one entry per
statement of *its own* body, so two of them cannot be interleaved — the surviving body keeps its
own table and the diagnostics that are not positional are unioned around it.

### `rcgjs-object/src/jir/conflict.rs`

The reusable half of the autopsy: `Verdict`, `Cause`, `ConflictRecord`, `first_difference`, `cause`,
`render`, and the deterministic JSON writer. `examples/jir-conflicts.rs` and the linker both call it,
so the tool that produced the numbers above and the file a build writes say the same things.

### `backend/src/driver.rs`

`js-jir=off` records `Decline::Encode(EncodeDecline::Disabled)` rather than letting the writer fall
back to `Synthesized`. The lane being off is a property of the build, not of the item, and a
coverage report has to be able to tell the two apart — otherwise turning the lane off reads as every
item in the program suddenly being unencodable.

### What the integration changed, measured

The FingerprintJS idiomatic boundary program, `js-opt=on`, built with the v2 compiler before and
after the rule (the only difference being `merge_choosing` in place of v1's `merge`):

| | items | unavailable | of which conflict | min ESM |
| --- | --- | --- | --- | --- |
| v1 (frozen oracle) | 1,188 | 83 | **74** | 258,721 B |
| v2, v1's merge | 1,191 | 83 | **74** | 259,527 B |
| v2, this rule | 1,191 | **9** | **0** | 259,527 B |

All 74 conflicts are gone; the nine that remain are the synthesized items, which have no typed MIR
body and never did. The program is **byte-identical** across the change: the 74 items become
available to the frozen v1 text optimizer and it lowers none of them differently, because its
native lowering is gated on much more than availability. The 806 B between v2 and v1 predates this
work — a v2 `js-opt=report` build already emitted 1,771 items to v1's 1,768 before part (b) began.

`js-opt=report` is the same picture with no dedup at all: 1,771 items, 9 unavailable, 0 conflicts.

Two other measurements worth recording:

* **`scripts/matrix.sh`'s `names-mangled` mode went 43/83 to 75/83.** That mode has been pinned as
  broken since wave 3, and the pin's own comment names the cause: "the pre-existing two-crate `core`
  path spelling defect", one `core` item spelled two ways across two crates raising v1's "two
  different definitions of ...". Keying on `SemanticKey` instead of on printed text is exactly what
  that defect needed. Every fixture in the mode now links; the eight that still fail are all
  `over-budget`, which is the mode's own cost.
* **`scripts/jir-coverage.sh --fixtures` now reads the lane report** rather than falling back to the
  optimizer report, and reports 7,573 items over 110 fixtures: 7,164 `semantic` (5,413,351 B, 99.1%
  of emitted bytes) and 409 `unavailable`, every one of them `encode.synthesized`. Nothing is
  `available`, which is correct for Phase 1 — no lane prints from a JIR body yet.

## What is not integrated

* The lane report's `available` state has no producer yet: Phase 1 prints from the baseline lane, so
  every item with a body is `semantic`. Phase 2's lowerer is what moves rows to `available`.
* `merge_duplicates` still falls back to v1's text comparison when either side has no summary — a
  format 19 object, or an item the link step synthesized. That path disappears with format 19.
* Source maps still decline the alias merge outright (`SourceMapDecline::Dedup`): an item's map
  entries are byte offsets into its own text, and redirecting a reference rewrites that text.
