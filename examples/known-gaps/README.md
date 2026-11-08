# Later recovery regression targets

These cases were reconstructed from the October audit conversation. Their `.expected` files are fresh native outputs, not recovered original byte-for-byte receipts. `PROVENANCE.json` retains that distinction. They must not be rebaselined to an incorrect JavaScript result.

Run `python3 scripts/check-known-gaps.py`. This is a **strict diagnostic**: it exits nonzero for any compile refusal, runtime exception, timeout, or output mismatch and writes each result to `build/known-gaps/receipt.json`. It is separate from the recovered September core suite so that a passing historical corpus cannot hide these later obligations. Some are known gaps, not approved target semantics.
