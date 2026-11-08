# contract

The dom-expressions contract: what the Rust to JS emitter must reproduce, extracted from pinned upstream sources by executable drivers rather than by hand.

Start with [`CONTRACT-DOM.md`](CONTRACT-DOM.md). Everything else here exists to keep that document honest.

[`PROCEDURES.md`](PROCEDURES.md) is the second contract page: the procedure call wire, which is Topcoat's own rather than upstream's, so it is cited to Topcoat's source and gated differently. See the fixture table.

[`JS-EXTERN.md`](JS-EXTERN.md) is the third: the JavaScript call shapes a `#[js_extern]` declaration lowers to. It has no upstream either, and no implementation yet, so it is gated differently again -- by driving a checked-in fake library through reference emissions and recording what they did.

## The idea

Every fact in the contract is produced by running the real upstream code and recording what it did. Nothing is transcribed by eye. That has one practical consequence worth stating up front: bumping a version in [`upstream.lock`](upstream.lock) and re-running the harness produces a diff, and **that diff is the drift report**. There is no separate step where someone re-reads upstream and hopes to notice what changed.

## Layout

| path | what |
|---|---|
| `CONTRACT-DOM.md` | the DOM contract itself, 69 numbered points across 15 sections (0 to 14) |
| `PROCEDURES.md` | the procedure call wire, shared by the server's HTTP tests and the client's fixtures |
| `JS-EXTERN.md` | the `#[js_extern]` call shapes, shared by the backend's lowering and the dashboard's chart bindings |
| `upstream.lock` | machine-readable pins: version, sha512, git sha, extraction date |
| `vendor/fetch.sh` | downloads and hash-verifies every upstream source |
| `vendor/package.json` | the pinned toolchain, exact versions, zero ranges |
| `harness/` | the extractors and proofs, one driver per fact |
| `fixtures/` | the recorded output, committed |
| `parity/` | the SSR to client hydration parity harness, a separate package with its own jsdom; also carries the island delivery's size budgets |

`vendor/upstream/` and `vendor/node_modules/` are gitignored. Everything needed to regenerate them is committed.

## Running it

```sh
cd vendor && ./fetch.sh && yarn install --frozen-lockfile && cd ..
node --import ./harness/register-loader.mjs harness/run-all.mjs --check
```

`--check` snapshots `fixtures/` before and after, so a clean run proves the committed fixtures are exactly what the pinned upstream produces today. Without it, the fixtures are simply regenerated. Fifteen steps run in dependency order. The last is `check-l2-status.mjs --check`, which is a checker rather than an extractor: it writes nothing, so it cannot perturb the snapshot, and it runs in check mode so a failure carries its own directional message instead of surfacing only as a changed hash.

Two fixtures cannot work that way, and they fail the same test for opposite reasons. Both describe something of Topcoat's own rather than something upstream, so there is nothing to re-derive them from; each is gated from a different other side.

`procedure-wire.json` describes a wire that already exists, so it is checked against the implementation: `check-procedure-wire.mjs` re-reads the framework source lines the file cites and fails if a cited value moved. `js-extern/vectors.json` describes call shapes that do NOT exist yet, so there is no implementation to cite. It is checked against a fake library instead: `check-js-extern.mjs` drives `fixtures/js-extern/chart-lib.mjs` through the reference emissions in `drivers.mjs` and records what they did, and every driver carries hand-written assertions about the shape it demonstrates so that the recording is not merely circular. No vector's assertions hold for any other vector's recording, which is what makes them claims about a shape rather than about a trace.

Individual extractors run the same way:

```sh
node --import ./harness/register-loader.mjs harness/extract-abi.mjs
```

[`parity/`](parity/README.md) is a separate package and runs on its own, because it tests the emitter against a live DOM rather than testing upstream. It is deliberately not part of `run-all.mjs`: it needs a built demo-app, and a drift gate that fails because a sibling crate is not built would be reporting the wrong thing.

```sh
cd parity && npm install && ./check.sh --check
```

Three fixtures there, all three now live against demo-app's own islands: `counter`, `nested` and `search`. Each was written as a `pending` scaffold before the island it measures existed, and a pending fixture is not a skipped one: everything about it that does not need the server's bytes is checked on every run, so its expectations are argued before the island lands rather than written in a hurry when it does. What that amounts to differs by fixture, and usefully so. `nested`'s declared key sequence was validated against `keys-nested.json` for two waves before its island existed, and when the island landed **every key held exactly**. `search` has no components and nothing to nest, so what was validated instead was its whole procedure wire against [`PROCEDURES.md`](PROCEDURES.md)'s vectors -- and when its island landed, the island's SOURCE shape had changed completely while every one of the fixture's assertions still held, because they are about the DOM and the bytes on the socket rather than about how the island holds its state.

`parity/lib/keys.mjs` is the nested-key half of the comparison library: it inverts the key encoding, so a `data-hk` can be read back into the chain of component contexts it was allocated through. `parity/lib/wire.mjs` is the procedure half: a fixture names the `procedure-wire.json` vectors it honours and this reads the values off them, so a fixture's fetch stub cannot drift from the spec -- which matters because one of those values has already been corrected once. `parity/budgets.json` records per-subject gzip size budgets for the island delivery, banded on both sides, because a chunk that unexpectedly shrank has lost something and a ceiling would call that a pass.

161 unit checks, of which the nested-key group runs against all 108 recorded oracle cases.

## What each fixture pins

| fixture | pins | driver |
|---|---|---|
| `abi.json` | the 48 client exports, arity, origin, and solid-js/web parity | `extract-abi.mjs` |
| `delegated-events.json` | the 22 delegated events, asserted not just recorded | `extract-constants.mjs` |
| `properties.json` | Properties, ChildProperties, BooleanAttributes, Aliases, SVGNamespace | `extract-constants.mjs` |
| `escaping.json` | 47 escape cases and 12 invariants, plus the Topcoat delta | `extract-escaping.mjs` |
| `keys.json` | 264 hydration-key triples and the prefix-free scheme | `extract-keys.mjs` |
| `keys-nested.json` | 108 component-nesting cases driven through both runtimes: which slot a component spends, what its child context id is, and server/client agreement per case | `extract-nested-keys.mjs` |
| `hydration-script.js` | the SSR bootstrap, with a substitutable event list | `extract-bootstrap.mjs` |
| `templates.jsonl` | 135 template trees and walk paths (51 hand-written + 84 from the corpus), 10 known-ambiguous | `gen-tree-oracle.mjs` |
| `corpus/` | 18 construct families: a `view!` fixture, its JSX equivalent, both compiled outputs, and a trace | `gen-corpus.mjs` |
| `markers.md` | the two marker syntaxes and getNextMarker's depth counting | hand-written from cited source |
| `reference-parity.json` | 43 upstream fixtures reproduced byte for byte | `verify-reference.mjs` |
| `reference-traces/` | 7 normalized runtime call traces | `run-trace.mjs` |
| `upstream/` | upstream's own fixture dirs, copied verbatim with sha256s | `import-upstream-fixtures.mjs` |
| `procedure-wire.json` | 19 procedure-call wire vectors: both media types, both zero-argument spellings, the error shapes | hand-written, source-checked by `check-procedure-wire.mjs` |
| `js-extern/vectors.json` | 17 foreign-call vectors across 6 shapes: construct, send, get, set, index, call, plus both arms of a nullable return | recorded by `check-js-extern.mjs` driving `js-extern/chart-lib.mjs` |
| `l2-status.json` | the per-family L2 verdict: 18 rows of MATCH / DIFFERS / EXCLUDED / GAP, with the attributed cause of every difference | recomputed and ENFORCED by `check-l2-status.mjs`, which fails on a regression AND on an improvement |

## Why the harness is shaped this way

**`harness/loader.mjs` exists because dom-expressions is not importable as shipped.** It publishes `src/` as raw ESM inside a package with no `type: "module"` and no `exports` map, and `client.js` imports from the bare specifier `rxcore`, a build-time alias rather than a package. The loader resolves `rxcore` to solid's real core, forces those files to load as modules, and anchors bare-specifier resolution at `vendor/node_modules`. Without it every extractor would have to parse source text instead of running it.

**`harness/rxcore.mjs` binds the seam to solid deliberately.** Upstream's own tests bind it to an `s-js` shim. We bind it to solid-js because solid-js is what the vendored runtime actually ships, so the arities recorded in `abi.json` are the real ones rather than stub artifacts.

**`harness/load-plugin-validate.mjs` runs upstream's validator, it does not reimplement it.** The template round-trip check is the one piece of upstream logic that decides whether a template is safe to walk. Its module wrapper is rewritten so node can load it; not one line of the validation logic is touched.

**`gen-tree-oracle.mjs` is fed from two directions.** Its hand-written cases are
chosen for the parser hazard each one probes, which biases the oracle toward
hazards and away from the ordinary. So `extract-templates.mjs` adds every
distinct template string the reference compiler actually emitted across the
corpus — parsed out of the compiled output, never regexed. Their
`templateWithClosingTags` form is rebuilt with a flat tokenizer rather than with
parse5, deliberately: deriving it by parsing and re-serialising would make
upstream's parse-and-re-serialise validation pass by construction. All 84 pass
the validator, which is the expected answer — the plugin runs the same check
before it commits to a template.

**`compare-trace.mjs` re-derives the normalization it depends on.** `trace.mjs`
already numbers nodes in first-appearance order, but that numbering is per-run,
so two emitters with the same call sequence in a different internal order still
disagree on the numbers. The comparator renumbers both sides before comparing
anything, and aligns records by longest common subsequence rather than by index —
index alignment reports one inserted record as "everything after this differs".
Accepted divergences live in a delta file; every rule needs a written `why`, and
a rule that matches nothing is reported rather than passing quietly.

**`run-trace.mjs` maps modules rather than rewriting imports.** Compiled fixtures import from `r-dom`; the loader points that at the recording stub. Rewriting the compiled text was the alternative and was rejected, because then the bytes being traced would no longer be the bytes `verify-reference.mjs` proved faithful.

## Reproducing upstream output is fussier than it looks

Byte matching upstream's committed fixture outputs needs more than the right plugin version:

- `@babel/core@7.20.12` with `@babel/traverse@7.23.2` and `@babel/generator@7.20.7` forced as yarn resolutions. `Scope.generateUid` lives in traverse and its numbering changed after 7.23, which renames every `_tmpl$` and `_el$` identifier. Pinning only `@babel/core` is not enough, because yarn hoists a newer traverse that still satisfies core's caret range.
- `prettier@2.8.2` and upstream's `.prettierrc`. babel-plugin-tester formats every fixture output through prettier before writing it, so the committed files are prettier artifacts.

Both are recorded in `upstream.lock` and enforced by `vendor/package.json`.

## Known deviations from the original spec

- **No `jsdom` in THIS tree.** Nothing the extractors do needs a live DOM: the tree oracle is parse5-only and the trace stub must deliberately avoid a real DOM. And jsdom 30 requires node `^24.15` (this environment is 24.5) and depends on parse5 8, which would put a second parse5 major here. The oracle must use the same parse5 the reference plugin uses, or the two can disagree about HTML parsing. Reasoning is recorded in `vendor/package.json`.

  Both objections are about this dependency tree, not about jsdom. [`parity/`](parity/README.md) needs a live DOM and a real MutationObserver, so it is a separate package with its own `node_modules` and lockfile, pinning jsdom 29.1.1 (the last line that admits node 24.5). `vendor/node_modules` keeps parse5 7.3.0 as the only parse5 the oracle ever sees.
- **`@babel/core` is pinned to 7.20.12, not the latest 7.x.** See above; the latest 7.x does not reproduce upstream's output.
- **Two extra harness files** beyond the specified list: `loader.mjs` plus `register-loader.mjs`, `paths.mjs`, `rxcore.mjs`, `load-plugin-validate.mjs`, `empty-module.mjs`, `verify-reference.mjs`, and `run-all.mjs`. These are the shared plumbing the specified extractors sit on.
- **`trace.mjs` exports two names that are not in `abi.json`.** `Show` and `For`. The plugin's `builtIns` option makes compiled output `import { Show } from "r-dom"` — from `moduleName`, which the dom-expressions client runtime does not export. In a real app solid-js/web re-exports solid's control flow alongside the DOM runtime and the seam closes; against a bare stub it does not. Documented at the definitions, and it cannot change an existing trace: no `RECORD_TARGETS` fixture imports either name.
