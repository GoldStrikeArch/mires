# Mires Continuous Differential Oracle

`oracle/` continuously verifies semantic equivalence between **native `rustc`**
and **`mires` (Rust → JavaScript)** across four complementary tiers of real
Rust code, using `cargo mires test --diff` so every test's pass/fail/ignore
outcome must match native `cargo test` test-for-test.

## Suites

| Suite | Location | What It Tests |
|---|---|---|
| **`coretests`** | [`oracle/coretests/`](coretests/) | Real upstream `$SYSROOT/lib/rustlib/src/rust/library/coretests/tests/` modules compiled unmodified via symlinks; skipped modules/tests are explicitly tracked in [`coretests/skiplist.json`](coretests/skiplist.json). |
| **`alloctests`** | [`oracle/alloctests/`](alloctests/) | Real upstream `$SYSROOT/lib/rustlib/src/rust/library/alloctests/tests/` modules compiled unmodified via symlinks; skipped modules/tests are explicitly tracked in [`alloctests/skiplist.json`](alloctests/skiplist.json). |
| **`rustc-ui`** | [`oracle/rustc-ui/`](rustc-ui/) | 52 curated `rustc/tests/ui`-style `run-pass` differential tests covering pattern matching, niches, closures, `dyn Trait` & supertrait upcasting, `Rc<str>`/`Arc<str>`, `Drop` & `catch_unwind`, `async` pollers, interior mutability, slices/`str`, collections (`BinaryHeap`, `BTreeMap`, `LinkedList`, `VecDeque`), const eval, and generics. |
| **`crates`** | [`oracle/crates/`](crates/) | 19 popular pure-Rust `crates.io` crates (`adler2`, `base64`, `bitflags`, `bytes`, `crc32fast`, `csv`, `flate2`, `fnv`, `hex`, `itertools`, `miniz_oxide`, `ordered-float`, `percent-encoding`, `pulldown-cmark`, `regex-lite`, `serde`/`serde_json`, `smallvec`, `unicode-ident`, `url`) pinned in `oracle/crates/Cargo.lock`. |

Compiler bugs and lowering limits uncovered while building and expanding the
oracle are catalogued with minimal repros in
[`oracle/CRATES-FINDINGS.md`](CRATES-FINDINGS.md).

## Running the Oracle

Build the Mires backend and `cargo-mires` once:

```bash
unset CARGO_INCREMENTAL
bash scripts/build.sh --locked
cargo build --release --locked -p cargo-mires
```

Run the **fast tier** (designed for PR CI — runs `rustc-ui`, a representative
`coretests` & `alloctests` slice, and a fast subset of `crates.io` suites in
`dev` profile):

```bash
node oracle/run-oracle.mjs --tier fast
```

Run the **full tier** (designed for scheduled daily CI — runs ALL `coretests`,
`alloctests`, `rustc-ui`, and ALL 19 `crates.io` crates in BOTH `dev` AND
`release-balanced` profiles):

```bash
node oracle/run-oracle.mjs --tier full
```

You can also filter to specific suites or profiles:

```bash
node oracle/run-oracle.mjs --tier full --only coretests,alloctests
node oracle/run-oracle.mjs --tier full --only crates --profiles dev
```

## Outputs

Every run writes:

- `build/oracle/receipt.json` — machine-readable per-suite, per-profile totals,
  durations, test counts, and differential parity status.
- `build/oracle/scorecard.md` — human-readable Markdown scorecard (published to
  `$GITHUB_STEP_SUMMARY` and uploaded as an artifact in
  [`.github/workflows/oracle.yml`](../.github/workflows/oracle.yml)).

Any compilation failure, runtime failure, or native-vs-Mires outcome mismatch
fails the runner with a non-zero exit code.

## Policy on Upstream Assertions

Upstream `library/coretests` and `library/alloctests` files are **never edited
or weakened**. The runner generates an ephemeral Cargo package under
`target/oracle-work/` that symlinks directly to the pinned toolchain's
`$SYSROOT/lib/rustlib/src/rust/library/{coretests,alloctests}/tests/` sources
and passes explicit `--skip` flags strictly from `skiplist.json`. As compiler
support improves, delete entries from `skiplist.json` to expand differential
coverage automatically.
