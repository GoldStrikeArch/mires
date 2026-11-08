# Wave 6 — contract/docs slice notes

Owner: contract agent (owns `contract/`, reads everything else).
Bank file, updated as work lands. Backend agent coordinates via its own wave6 bank.

## Status

- [ ] 1. L2 failable gate (`fixtures/l2-status.json` + harness check step)
- [x] 2. Refresh six stale documents — done, and it was EIGHT, not six
- [x] 3. Resolve `deltas.json` orphan — reduced to a pointer, kept for its citations
- [x] 4. Keyed-list resolution — backend DOCUMENTED; assertion stays inverted, re-cited
- [x] 5. `contract/G2-FINAL.md` — COMPLETE. 687 lines, 9 sections, no placeholders.

## The gate's first real catch

Worth the top of the file. The backend's wave-6 sweep ran `contract/parity/check.sh` and it
STOPPED at the new gate, unprompted:

    IMPROVED      08-fragments: committed GAP, measured DIFFERS
    UNATTRIBUTED  08-fragments DIFFERS with no `attribution`

Family 08 closed that same wave and had no row yet. The tripwire in "an EXCLUDED/GAP row must
really be uncompared" fired on the exact family it was designed for — and it fired on an
IMPROVEMENT, which is the half of the design most likely to be argued with. Row recorded with
`["fragment-roots-must-be-elements", "memo-for-a-fragment-member"]`; measured DIFFERS, 7
differences, reference 10 records, ours 15, matching the backend's report exactly.

Final L2 state: **1 MATCH, 13 DIFFERS, 2 EXCLUDED, 2 GAP.**

## Final verification, this slice

    node --import ./register-loader.mjs run-all.mjs --check   15/15 steps green, NO DRIFT
    node --import ./register-loader.mjs check-l2-status.mjs   13 DIFFERS, 2 EXCLUDED, 2 GAP, 1 MATCH
    node contract/parity/test.mjs                             176/176
    node contract/parity/dts.mjs --check                       6/6, incl. tsc --noEmit over 4 files
    JSON revalidated, check.sh sh -n, all .mjs node --check

## Log

- Session start: reading G2-CHECKLIST.md + wave-5/6 banks. Backend's wave-6 bank
  (`build/logs/wave6-backend-report.md`) is at STARTED, all six items pending — no keyed
  decision and no 08/09/10 yet. Proceeding with items 1/2/3, polling for 4/5.

### Item 1: the L2 gate — LANDED

Two files:

- `contract/fixtures/l2-status.json` — the committed verdict, one row per corpus family.
- `contract/harness/check-l2-status.mjs` — recomputes it and fails on ANY difference.

**The load-bearing design decision: it compares the COMMITTED trace, not a freshly emitted
one, so it needs no build.** `scripts/dom-test.sh` already diffs the emitted trace against
`examples/dom-tests/NN.trace.expected` and fails the suite on any difference (its step 2). So
on a green dom run the committed trace IS the emitted trace, and comparing the committed one
against `contract/fixtures/corpus/<family>/expected.reference.trace` yields exactly the verdict
dom-test.sh prints — in about a second, with no rustc, no cargo, no backend build.

PROVEN, not assumed: recomputing families 02, 13 and 07b from the committed traces reproduces
`build/logs/dom-*.l2.log` from the last real dom run byte for byte, the only difference being
the `b:` path line naming `examples/dom-tests/NN.trace.expected` instead of
`build/domtest/NN.trace`.

Consequence worth stating: this gate cannot catch an emitter change on its own — dom-test.sh
catches that first. It catches what dom-test.sh cannot: a re-baselined trace that silently moved
a family's L2 verdict, and a reference trace or delta rule that moved underneath one.

Row shape: MEASURED fields (`state`, `differences`, `accepted`, `staleRules`, `causes`,
`records`) are rewritten by `--record` and compared by a check; DECLARED fields (`attribution`,
`why`, `note`) are hand-written and carried forward untouched. `causes` is a mechanically derived
signature — the distinct `kind:op` pairs, sorted — coarse enough to be stable and sharp enough
to change when the shape of the disagreement changes.

Enforced, beyond equality:

- a `DIFFERS` row must carry `attribution`, and every name in it must be a key of
  `$whatIsDeliberatelyNotHere` in `examples/dom-tests/deltas.json`. That is what makes
  "ATTRIBUTED" mechanical rather than a word in a table.
- an `EXCLUDED`/`GAP` row must carry `why`, and must really be uncompared: if a `NN.family`
  starts naming it, the declaration is stale and the check fails. **This is the tripwire for
  the backend landing 08/09/10** — the moment a fixture appears, the GAP row fails with
  "committed GAP, measured DIFFERS/MATCH" and the real status has to be recorded.
- a `MATCH` row may not load a delta rule that matched nothing.

Direction is reported: `REGRESSED` | `IMPROVED` | `NEW` | `REMOVED` | `UNATTRIBUTED` |
`UNKNOWN-CAUSE` | `UNJUSTIFIED` | `STALE-RULE`. **An improvement fails too**, with the
`examples/emit/NN.maxbytes` precedent named in the message: a budget you beat is a budget you
re-baseline, not one you leave loose.

Recorded state: **1 MATCH, 12 DIFFERS, 2 EXCLUDED, 3 GAP.** Matches the G2 checklist's
1/12/2/3 exactly, which is the audit's own table now made machine-readable.

Negative controls, all six tripped and restored:

    IMPROVED  01-simple-elements: committed DIFFERS, measured MATCH
    REGRESSED 02-text-interpolation.differences: committed 24, measured 25
    IMPROVED  05-conditional: committed GAP, measured DIFFERS
    IMPROVED  06-insert-children.differences: committed 99, measured 20
    UNATTRIBUTED  05-conditional / 13-topcoat-for
    UNKNOWN-CAUSE 14-topcoat-local cites "no-such-cause"

### Items 2 + 3: documents refreshed — the audit named six, there were EIGHT

The two new ones were found by reading the code rather than the audit, and both were claims that
the code had already falsified:

- `16-topcoat-spread/NOTES.md:3` + `view.rs:3` said attribute spread is NOT-YET-LOWERABLE and
  `HoleKind::Spread` is "never constructed". It IS constructed (`lower/attributes.rs:23`). The
  family is EXCLUDED from L2 for a different reason entirely — the spike has no type to write a
  spread's VALUE with — and the docs now say that instead.
- `17-topcoat-signal/NOTES.md:3` + `view.rs:3` said `lower/signal_declaration.rs` "emits nothing at
  all" and "drops the initializer expression", so "the signal has no storage". It emits
  `let <ident> = ::view_abi::signal(<ordinal>, <init>);`. Storage exists.

A third correction, partial rather than wrong: `03-attribute-expressions/NOTES.md:53` said the
name-to-sink table is not lowerable and `HoleKind::Property` is "never constructed". The table
landed (`view-dom/src/contract.rs::is_property`) and the kind IS constructed — but only from
`lower/bind_attribute.rs`. `lower/attribute.rs` still emits `HoleKind::Attribute` unconditionally,
so `value=(v)` reaches `setAttribute` while `:value=$(v)` reaches the property sink. Now stated
that way.

Refreshed, all under `contract/fixtures/corpus/`:

| document | was | now |
|---|---|---|
| `README.md` families table | a `view-dom status` column, two waves stale | column REMOVED, replaced by a pointer to `l2-status.json` + the command to check it |
| `README.md` L0 list | an enumeration of which families compile | pointer; the enumeration was live status |
| `README.md` L2/L1 intent table | read as status | kept, relabelled DESIGN INTENT, with the two intents measurement has since settled (05 closed, 13 cannot) |
| `README.md` NOT-YET-LOWERABLE ledger | listed `if`/`match`/`for`/`let`/block/bind/spread as blocking errors | rewritten from the `unsupported`/`error` call sites; retitled "What `view-dom` refuses"; keeps a paragraph saying what it USED to claim, so a reader can tell refreshed from stale |
| `README.md` accepted deltas | "Five rules are seeded today… nothing measured yet" | says which file the suites actually load, and what each seeded rule measured to |
| `{11,12,13,14,15}/NOTES.md` line 3 | "**NOT-YET-LOWERABLE.**" | "**LOWERS.**" + what IS still refused + a pointer, never a restated status |
| `{11..17}/view.rs` module docs | same claim in Rust doc comments | same treatment |
| `{05,11,12,13,14,15,16}/NOTES.md` "view-dom features needed" | NOT-YET-LOWERABLE bullets | LANDED / STILL REFUSED / DELIBERATELY NOT COMING |
| `03/NOTES.md` | property sink "never constructed" | partial: constructed from bind only |

The recurrence fix, which is the checklist's own lesson applied: **no corpus document restates a
per-family status any more.** Every one of them points at `contract/fixtures/l2-status.json`, which
is the only place the status lives and is now enforced.

`contract/fixtures/corpus/deltas.json` (item 3) is KEPT, not deleted, and reduced to a pointer.
Deleting it would have broken two live citations: `examples/dom-tests/deltas.json` cites four of
its rules by id as "an ATTRIBUTED COPY of the rule of the same id" there, and `CONTRACT.md:1557`
cites its prediction for family 07. Its `$what` now says it is a design record, a new `$notLoaded`
block names the live file and the live status file, and `$status` — which said "Seeded, not
measured. The Rust emitter does not compile these fixtures yet" — now records what each seeded
rule actually measured to.

TWO STALE DOCS ARE NOT MINE TO FIX and are carried into G2-FINAL.md as required corrections:
`VERDICT.md:182` (the family-07 parenthetical) and `CONTRACT.md:426` ("28 runtime fixtures and 13
JS goldens"; now 46 and 20). `CONTRACT.md` is backend-owned, `VERDICT.md` is the coordinator's.

### Item 4: keyed-list — backend DOCUMENTED, so the current state stands

Backend banked the decision: **`push_keyed` stays identity-only, the dashboard is NOT re-keyed, the
inverted assertion STAYS inverted.** No re-inversion needed. What I did instead:

- wrote the flip history into `contract/parity/fixtures/dashboard/interact.mjs`'s header —
  FLIP 1 (scaffold to keyed), FLIP 2 (keyed to inverted, with the measured 4310-after-4460), and
  NOT FLIPPED (wave 6) with the three findings — so a third flip has to read why the second one
  was not undone;
- added the second cause to the section-4 check comment and to `fixture.json`'s `expected.$notKeyed`;
- re-cited family 13 in `l2-status.json`'s `note` and in `corpus/13-topcoat-for/NOTES.md`.

The finding worth carrying to the verdict is the backend's K3: **no version of `push_keyed` can
move family 13 to MATCH**, because the reference `<For>` clones a row's template once per row EVER
and an eager Rust loop clones once per row PER RENDER, before any reconcile is reached. So the OUT
entry for keyed `for` is a design consequence, not a deferral waiting on a fix. Their K1 also
sharpens the rule the checklist stated loosely: a keyed row's changing parts must be reactive holes
OF THE ROW'S OWN TEMPLATE; a plain `( )` hole in a keyed row is `Fill::Once`, written to a node the
reconcile discards.

New finding, banked while attributing family 03: one of its differences is an artifact of the
`template-attribute-quotes` delta RULE, not of either emitter. The reference writes the boolean
attribute bare as `disabled`, view-dom writes `disabled=""`, and the rule rewrites that to
`disabled=`. A clause for the empty value would close it. `deltas.json` is backend-owned so it is
recorded as a `note` on the row rather than fixed here.
