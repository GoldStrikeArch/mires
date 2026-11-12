# Round 2 open regression inventory

These are original verifier programs copied into the repository before temporary worktrees are removed. `PROVENANCE.json` records their sources and hashes. Their `.expected` files were independently generated with the pinned native Rust compiler at optimization levels 0 and 3; both outputs agreed. These are **targets to fix**, not accepted wrong-answer goldens.

Run `python3 scripts/check-round2-open.py --mode both` after building the compiler. It runs cleanup-on release-balanced, and the same profile with rustc `-Oz` and MIR inlining. It builds matching sysroots, preserves logs and receipts under `build/round2-open`, fails on empty/missing inputs, and exits nonzero while any case is incorrect. It does not modify expectations.

| Case | Obligation | Current classification |
| --- | --- | --- |
| `large_enum_copy` | Copying an enum payload must not alias the original payload just because there are many variants. | Open; A8 |
| `array_rest_write` | A mutable rest-pattern view writes through to the original array. | Open; A8 |
| `static_projection_cycle` | Pointers to first fields of statics preserve their target and identity, including cycles. | Open; A7; a load failure becoming a silent wrong answer is not fail-closed. |
| `static_proto_field` | A Rust field named `__proto__` is data, not JavaScript literal prototype syntax. | Open; A7 |

The stable test runner deliberately lives outside the historical archive. Historical generators and receipts live in `research/v2-restoration/rounds-1-2/`; raw/private evidence is under `recovery/worktree-preservation-*` and is not public Git content.
