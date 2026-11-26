# JIR body identity and merging

A monomorphized Rust item can reach `jsc-link` more than once: two crates can codegen the same generic instantiation, two codegen units of one crate can emit it, or `link_dedup` can prove that two differently named items lower to identical JavaScript and merge them. Each case needs a deterministic rule for which JIR body survives.

## Two merge relations

The linker separates true duplicates from link-dedup aliases because they have different soundness rules.

### 1. Duplicates (the same item in multiple objects)

Each item is identified by `SemanticKey = (link_name, Option<ItemId>)`, pairing the emitted JavaScript name with the stable Rust identity (`DefPathHash` plus a hash of the generic instantiation). When two objects provide the same `link_name`, `jir::summary::merge_duplicates(a, b)` decides how to merge them:

| Situation | Decision |
| --- | --- |
| Object headers disagree (`merge_headers`: JIR schema, `rustc` version, data layout, or emit options) | Hard link error naming both objects and the required rebuild |
| Different `SemanticKey`, same `link_name` | `Error(NameCollision)` (two distinct Rust definitions collided on one JS name) |
| Same key, equal normalized `semantic_hash` | `Merge`: keep either body and union diagnostic metadata |
| Same key, body present on one side | `Available(side)`: keep the side with a body |
| Same key, different normalized hashes | `Choose { keep, chosen, rejected }`: keep the smaller `semantic_hash` and record `Decline::Merge(BodyChoice { chosen, rejected })` |

Picking the smaller hash keeps the result independent of the order in which object files or rlibs were read.

### 2. Aliases (different items with identical JavaScript)

`jsc-link/src/link_dedup.rs` merges items whose printed JavaScript is identical under different names, such as two monomorphizations of a generic function whose type parameters erase to the same JS code, and redirects references to a single winner. Because their type arguments differ, their `SemanticKey`s and JIR bodies also differ legitimately.

`jir::summary::merge_aliases(a, b)` applies the same deterministic min-hash body choice without requiring matching `SemanticKey`s. In practice, many `core` and `alloc` instantiations (such as `drop_glue`, `array::IntoIter::drop`, and `Pin::new_unchecked`) share identical JavaScript across distinct types; keeping alias merging separate from duplicate checking avoids false conflicts on those winners.

## Body normalization

Two codegen units can emit semantically identical MIR whose raw encoded bytes differ because of `StorageLive`/`StorageDead` markers, block numbering, or local intern IDs. `jir::summary::NormalizedBody::of(body, interns)` converts a body into a canonical form, and `semantic_hash(body, interns)` hashes that canonical form with SipHash-1-3-128 using fixed keys (`Hash128`, never `DefaultHasher`, because hashes are stored in object files).

Normalization applies seven rules:

1. **Drop storage markers and nops.** `StatementKind::{StorageLive, StorageDead, Nop}` have no runtime effect in JavaScript lowering.
2. **Collapse borrowck-only edges.** `Terminator::FalseEdge { real, .. }` and `Terminator::FalseUnwind { real, .. }` become `Goto(real)`.
3. **Renumber blocks in reverse postorder** from the entry block using the successor edges left after step 2. Unreachable blocks are dropped, so a CGU that left a dead block behind matches one that removed it.
4. **Renumber locals.** The return local and parameters keep declaration order (matching the JS ABI); remaining locals are numbered in first-mention order during the step 3 walk, and unused locals are dropped.
5. **Exclude origins.** Source spans live in `OriginTable`, outside `Body`. The one semantically visible location, `Call::track_caller`, stays in the body as a `Location` value.
6. **Re-intern strings, byte blobs, and source facts by content** in first-mention order so crate-local intern IDs compare equal across CGUs.
7. **Keep `TypeId`s as-is.** Every `TypeId` is already a global stable hash.

Because the normalized form is still a valid `Body`, diagnostic tools can walk two normal forms side by side and report the exact first field where they diverge.

`ItemSummary` stores two hashes for different jobs:

- `body_hash` — hash of the raw postcard bytes for blob addressing and integrity verification in object section S3.
- `semantic_hash` — hash of the normalized body, stored in the eagerly read S2 summary index so duplicate resolution never needs to inflate bodies just to compare them.

## Inspecting conflicts

`rcgjs-object/src/jir/conflict.rs` implements `first_difference`, `Cause`, and JSON/text reporting shared by both `jsc-link` (`-Cllvm-args=js-jir-dump-conflicts=<path>`) and a standalone CLI example:

```sh
cargo run -p rcgjs-object --example jir-conflicts -- \
  [--limit N] [--expect <names>] [--quiet] <file.rlib|object>...
```

The tool checks object headers, groups items across objects by JS name, and classifies each multi-object group as:

- `identical` — identical postcard bytes
- `normalized-identical` — different raw bytes, identical normalized form (merged cleanly)
- `different` — different normalized forms, printing the first differing field and classifying the cause as `inlined`, `shape`, or `content`
- `name-collision` — two distinct Rust items mapped to one JS name (link error)
- `no-body` — missing JIR body

## Linker integration

- `jsc-link/src/link.rs` validates headers once per object in `Read::extend`, resolves duplicate items via `merge_duplicates` in `dedup` (preserving entry-point linkage when adopting a body from an rlib), writes per-item availability to `-Cllvm-args=js-lane-report=<path>`, and optionally dumps conflict records via `-Cllvm-args=js-jir-dump-conflicts=<path>`.
- `jsc-link/src/link_dedup.rs` calls `choose_alias_body` (`merge_aliases` plus conflict recording) when merging identical printed items. Source maps still disable `link_dedup` because map offsets refer to the original item text before reference rewriting.
- `rcgjs-object/src/item.rs` keeps the winning body's positional `OriginTable` and unions non-positional diagnostic metadata around it.
