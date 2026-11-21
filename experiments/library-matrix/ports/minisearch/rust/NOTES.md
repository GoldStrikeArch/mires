# MiniSearch 7.2.0: Rust port (mires)

A faithful port of the subset in `libraries/minisearch/CONTRACT.md`: the same algorithms and data
structures as `minisearch/dist/es/index.js`, method by method, in safe `no_std` Rust (`#![deny(unsafe_code)]`,
no `js!{}`). It reaches exact parity (doubles included) on all 58 corpus cases in Node, Chrome and
Firefox.

| File | What it is |
| --- | --- |
| `src/searchable_map.rs` | `SearchableMap`: the radix tree, nodes as insertion-ordered entry lists (the `''` leaf among them), `createPath`, `lookup`, `remove`/`cleanup`/`merge`, `atPrefix`, the `TreeIterator` order, the Levenshtein matrix walk of `fuzzySearch` |
| `src/index.rs` | the `MiniSearch` class: add/remove/discard/replace, field lengths, vacuuming, search (option merging, query trees, `executeQuerySpec`, `termResults`, BM25+, the combinators, result assembly), autoSuggest, `toJSON` |
| `src/options.rs` | option types and their spread-merge (`{ ...a, ...b }`) |
| `src/text.rs`, `src/separators.rs` | UTF-16 strings: the default tokenizer, `toLowerCase`, the `Object.keys` order of `match` |
| `src/boundary.rs` | the corpus input into Rust values, the result back to host objects |

## Decisions

- **Strings are UTF-16** (`Vec<u16>`/`&[u16]`) everywhere the index sees them, read with
  `charCodeAt` at the boundary: MiniSearch splits radix edges, measures lengths and computes edit
  distances on code units, and lone surrogates must survive. Names (fields, options) are Rust
  `String`s; string ids are `String`s (the contract keeps ids well-formed).
- **Tokenizer and lower-casing are native.** The separator set is the 203-range table the corpus
  generator reads off the host `RegExp` (`libraries/minisearch/separators.json`, Unicode 17.0);
  lower-casing is `str::to_lowercase` (core's Unicode 17.0 tables, Final_Sigma included) applied to
  each well-formed run between lone surrogates. The contract also allows the host `RegExp` and
  `toLowerCase`; this port does not use them.
- **`Math.log` is the host's** (`js_bindings!`, `Math.log`). It has to be: engines disagree in the
  last bit (CONTRACT.md, "Floating point").
- **Data structures.** Radix nodes own their children (`Vec<Entry>`, labels `Box<[u16]>`). A posting
  list is a vector sorted by short id, which is exactly the insertion order of MiniSearch's
  `Map<shortId, freq>` (a document only ever adds postings under the newest short id). Per-document
  tables (`_documentIds`, `_fieldLength`, `_storedFields`) are vectors indexed by short id, whose
  ascending order is those `Map`s' insertion order. Result sets (`Map<shortId, result>`) and the id
  maps are `mires_js::collections::HashMap`, a host `Map`, insertion-ordered like MiniSearch's.
- **Mutation during iteration.** MiniSearch deletes terms while a `TreeIterator` walks the tree
  (vacuum, and search's cleanup of discarded postings). Its iterator snapshots each node's keys and
  only the term just visited is ever deleted, so the walk visits exactly the terms of the tree as it
  was when it began. The port therefore collects the vacuum's terms first, and applies search's
  postings cleanup in three places: the exact term's right after it is scored (before the prefix
  and fuzzy walks start, as in MiniSearch, because the merge it may cause reorders those walks),
  the prefix walk's after the whole walk (each prefix term only reads its own postings, so the
  deferred deletions, applied in walk order, leave the same index), and each fuzzy term's right
  after it is scored. The prefix-match deletion from the fuzzy results is done as one filter (the
  prefix matches are exactly the index terms extending the query term).
- **Vacuuming in a synchronous run** follows the contract: the first vacuum runs synchronously up to
  its first batch boundary and `isVacuuming` then stays true for the rest of the run.
- **`removeAll(null)`** (an explicit null argument) is MiniSearch's error, not `removeAll()`.
- **No cross-call state.** Every `run` builds a fresh index from its own input; nothing is cached
  between calls or keyed on input identity, and the input is never written.

## mires bugs met

The first version of this port met four compiler bugs. They are `MIRES-001` to `MIRES-004` in
[`MIRES-BUGS.md`](../../../MIRES-BUGS.md), each with a repro under
[`mires-bugs/MIRES-00N-*/`](../../../mires-bugs/). All four are fixed in the compiler, and the port
no longer works around any of them, so its sources carry no `WORKAROUND(MIRES-…)` tag:

- **`MIRES-003`** (`into_iter().map(..).collect()` refused in alloc's in-place collection):
  `index.rs` `auto_suggest` collects that way again. `index.min.js` is 2.8 KB larger than with the
  push loop it replaces.
- **`MIRES-002`** (`Box<[T]>::from(&[T])` read the wrong elements and could not be freed): radix
  labels are made with `Box::from` (`searchable_map.rs` `create_path`).
- **`MIRES-004`** (`<` on slices refused, and with it `sort_unstable` of `&[u16]` keys):
  `index.rs` `unique_count` still sorts positions through the shared `stable_order`, comparing
  with `Ord::cmp`. That is the size decision under "Size", not a workaround: `sort_unstable` plus
  `dedup` builds and passes now, but its second sort instantiation adds 13.4 KB.
- **`MIRES-001`** (`<(A, B) as Clone>::clone` miscompiled): nothing in the port clones a tuple.
  The vector-valued options stay `Rc<[T]>` on their own merit: the spread merge at every
  query-tree node then shares each value instead of copying it, as spreading a JavaScript object
  does. Search hits refer to stored fields by short id, and `match` entries are moved into the
  hit; neither needs a copy.

## Size

`dist/index.min.js` is 322,184 bytes (74,845 gzip-9, 60,023 Brotli-11). The first build, before the
release profile defaulted to `js-str=bytes`, was 318,772 bytes (74,318 gzip-9, 59,667 Brotli-11).
The current compiler adds 434 bytes to the unchanged source, and dropping the MIRES-002 and
MIRES-003 workarounds adds 2,978 more. The figures the matrix reports are in
`build/library-matrix/sizes.json` and `MATRIX.md`. By the optimizer report (about 352 KB of
program, in Rollup bytes before Terser), about 21 KB is Unicode lower-casing (core's conversion
tables, `Cased`/`Case_Ignorable` for Final_Sigma, `to_lowercase`). The single stable-sort
instantiation is 14.9 KB: `alloc::slice::sort_by`, its closure and the 23 `core::slice::sort`
items. `core::fmt` reached from panic paths is 6.6 KB: `Vec::remove`'s
`assert_failed` formats its indices, and `Rc<[T]>`'s layout calls `unwrap`. The rest is the port
and its generic instantiations (`Vec`, iterator adapters, `Option`, drop glue, the host-map
wrappers).

Every sort goes through one `sort_by` on positions (`index.rs` `stable_order`). Under mires each
instantiation of core's driftsort is 13–15 KB of JavaScript. The optimizer report of the
2026-11-21 build gives 14.9 KB here and 13.8 KB in d3-hierarchy's Rust port. Five element types
(hits, suggestions, `match` entries, tokens, serialized fields) cost 89.5 KB before this was
shared, on the compiler of the time.

## Performance

Measured by the harness (`tools/bench.mjs`, receipts under `build/library-matrix/bench/minisearch/`).
What the profiles show the time goes to, and what was changed for it (each change is ordinary
Rust):

- heap-block bookkeeping of the mires runtime (`__rt` side table lookups on every `Vec` deref,
  block registration on every allocation) and the garbage collector dominate: allocations were
  removed from the hot paths (one lower-casing buffer per field, field text borrowed, result terms
  shared as `Rc<[u16]>` per scored term instead of copied per document, output strings built on the
  host from code units instead of through a Rust `String`);
- radix labels are `Box<[u16]>`: reading `label[0]` of a `Vec` while scanning a node's children cost
  a side-table lookup per child;
- `u128` is a BigInt under mires: the ASCII separator set is four `u32` words;
- `Range::contains` builds a range object per call: the hot text paths compare directly.
