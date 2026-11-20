# Rust hash port

Each call owns its SHA-2, Keccak, BLAKE2 or HMAC state. The incremental path retains the partial block across updates and applies padding only when the caller requests the digest. SHA-512, BLAKE2b and Keccak represent each 64-bit lane as two 32-bit words.

Three structural differences from noble change no output:

- **The carry formula.** The carry out of the low half is an unsigned comparison. `Word::add` in `src/sha2.rs` adds `(lo < self.lo) as u32`, and `Word::add` in `src/blake2.rs` takes `overflowing_add`'s carry. The multi-term sums chain `add`. noble instead sums the unmasked low halves as a JS number and adds `(low / 2 ** 32) | 0` to the high half.
- **One BLAKE2 mix.** BLAKE2b and BLAKE2s share one compression parameterised by width: `mix`, `compress`, `Word::rotate`, `iv` and `load` take `wide`, and BLAKE2s leaves `Word.hi` zero. noble has separate `G1b`/`G2b` and `G1s`/`G2s`.
- **No one-call digest.** `matrix_run` feeds each chunk to `State::update`, `repeat` times, then calls `digest`, as `reference.mjs` drives `create().update().digest()`.

A `null` option, outside the contract's domain, is read as present. BLAKE2 rejects it and the other hashes ignore it, where npm's `Uint8Array.from(null)` throws for a null `key`, `salt` or `personalization`. A null `repeat` throws, where npm runs no updates.

`WORKAROUND(MIRES-020)` in `src/sha2.rs` passes the SHA-512 state slice to a named function. The original capturing closure returning the word pair fails in mires during call lowering. The [standalone reproducer](../../../mires-bugs/MIRES-020-closure-spread-cleanup/) includes the expected compiler diagnostic.
