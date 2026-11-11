# Round 1 adversarial regression inventory

These five small Rust programs preserve the independently reproduced round-1 failures. They are separate from the six October-derived cases in `examples/known-gaps/`. Their native expected outputs are retained without weakening them. A newly passing case is progress; a nonzero exit from this runner is an open correctness result, not an expected-success test.

```sh
bash scripts/build.sh --locked
python3 scripts/check-round1-open.py
JSC_RUST_OPT_LEVEL=z JSC_RUST_MIR_INLINE=on python3 scripts/check-round1-open.py
```

The default is cleanup-on `release-balanced`. Keep the receipt from each configuration separately before another run. The second command tests the MIR/optimization configuration used by the compact package. No private Fingerprint Pro sources are needed.

| Case | Original issue |
|---|---|
| `address_fields` | Runtime pointer recognition reads ordinary struct fields named `pr` and `off`. |
| `copy_alias` | Writing a by-value copy changes the source or an operand. |
| `flattened_safe` | A flattened array view reads past row zero incorrectly. |
| `lost_write` | A field store through an alias is discarded by scalar replacement. |
| `niche_constant` | Niche-optimized `Some` values in constant data are read as `None`. |

`PROVENANCE.json` records the original audit observation, not a promise that a different compiler/configuration has the same result. See the root `round_1_integration_report.md` for this promotion's actual outcomes.
