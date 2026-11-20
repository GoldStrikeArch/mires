# @noble/hashes 2.4.0: Kotlin/JS port

This port is written by hand. It replaces the port that `build/library-matrix/dev-noble-sha2/imperative.py`, `dev-noble-sha3/make_imperative.py` and `dev-noble-blake/jvm.py` generated. It ports the `sha2`, `sha3`, `blake2` and `hmac` modules of @noble/hashes 2.4.0, as far as `libraries/noble-hashes/CONTRACT.md` covers them, to Kotlin/JS 2.4.10. It follows noble's modules and classes.

| File | What it is |
| --- | --- |
| `src/U64.kt` | noble's `_u64.ts`: `shrSH`/`shrSL`, `rotrSH`/`rotrSL`, `rotrBH`/`rotrBL`, `rotr32H`/`rotr32L`, `fromNumH`/`fromNumL`, `add`, and `add3` to `add5` |
| `src/Utils.kt` | the `Hash` interface, `anumber`, `words` (unsigned literals to an `IntArray`), the big- and little-endian word reads and writes, `bytesToHex` |
| `src/Md.kt` | `_md.ts`: `chi`, `maj` and the abstract `HashMD` |
| `src/Sha2.kt` | `sha2.ts`: `Sha256` (SHA2_32B) and `Sha512` (SHA2_64B) |
| `src/Sha3.kt` | `sha3.ts`: `keccakP` and the `Keccak` sponge |
| `src/Blake2.kt` | `blake2.ts` and `_blake.ts`: `BSIGMA`, `Blake2Opts`, `checkBlake2Opts`, the abstract `Blake2` (noble's `_BLAKE2`), `Blake2b`, `Blake2s` |
| `src/Hmac.kt` | `hmac.ts`: `Hmac` |
| `src/Main.kt` | the boundary: `reference.mjs` in Kotlin |

## Boundary

The input is the corpus object itself, read through an `external interface` with nullable fields. Each byte array is copied into a `ByteArray` (`Uint8Array.from` in `reference.mjs`). `hasher` is `reference.mjs`'s `hashes` table plus HMAC, so an unknown name throws before any option is read. A one-shot `bytes` message is converted and then hashed with `create(opts).update(message).digest()`. Otherwise the state is created first and every chunk is converted, then the chunks are fed `repeat` times. This matches the order of `reference.mjs`. There is no one-call `digest(chunks)`: the boundary drives `update` once per chunk, as `reference.mjs` does. The digest returns as a `String`.

## How it follows noble

- **64-bit words are two 32-bit `Int` halves** in every SHA-512 and BLAKE2b variable and array, as in noble's `_u64.ts`. Shifts and rotations are noble's helpers and return one half.
- **The carry formula.** `add` and `add3`...`add5` return both halves as a `Pair`. Each carry out of the low halves is `sum.toUInt() < addend.toUInt()`, taken after every wrapped 32-bit addition. noble instead sums the unmasked low halves as a JS number and adds `(low / 2 ** 32) | 0` to the high half. The two give the same 64-bit sum. noble splits each multi-term sum into `addNL` and `addNH`; `addN` here computes both halves at once.
- **`HashMD`** keeps noble's `update` and `digestInto`. A whole block with nothing buffered is compressed straight from the input; otherwise bytes are buffered. Padding is `0x80` and zeros, with an extra block when the length field does not fit. The length field is the low 64 bits of the bit length. `Sha256` holds noble's `A`...`H` fields and `Sha512` its `Ah`, `Al`...`Hh`, `Hl` fields, and `process` is noble's.
- **`Keccak`** keeps noble's `keccakP` (theta, the single rho-pi cycle, chi with the saved first four words, iota) and its `update`/`finish`/`writeInto` logic on `state32`.
- **BLAKE2b and BLAKE2s do not share a mix.** BLAKE2b has noble's `G1b`/`G2b` on a 32-word work vector of halves (noble's `BBUF`); BLAKE2s has `G1s`/`G2s` on 32-bit words. Both sit on `Blake2`, noble's `_BLAKE2`. Its `update` withholds the last block, because the final block is compressed with a flag. The constructors validate in noble's order: `anumber`, then `checkBlake2Opts` for `dkLen`, `key`, `salt` and `personalization`. They then apply the parameter block, salt and personalization, and absorb the zero-padded key as the first block.
- **`Hmac`** is noble's `_HMAC` over any `Hash`. A key longer than a block is first replaced by its digest.

## Deviations from the npm source

None of them changes a result in the contract's domain.

- **Scratch arrays belong to each hash.** These are the SHA-256 and SHA-512 message schedules, BLAKE2's work vector and Keccak's theta columns. noble shares one module-level copy of each (`SHA256_W`, `SHA512_W_H`/`SHA512_W_L`, `BBUF`, `B`).
- **No zeroisation, no lifecycle flags, no `clone`.** noble wipes buffers after use (`clean`, `destroy`) and checks `finished`/`destroyed`. Here each state is digested once per call and dropped.
- **Keccak absorbs into 32-bit words directly.** noble XORs bytes through a `Uint8Array` view of `state32`. Here each byte is shifted into its word. noble takes the word fast path only for 4-byte-aligned input; here every block absorbed with nothing buffered takes it. The result is the same either way.
- **BLAKE2's whole-block path has no alignment test.** noble's zero-copy path also requires a 4-byte-aligned `byteOffset`. Here words are read from the bytes, so the test is unnecessary.
- **BLAKE2b and Keccak state as arrays.** BLAKE2b's eight words are an `IntArray` of halves (`v0l, v0h, ...`) where noble has sixteen fields. BLAKE2s works on a 16-word `IntArray` by index, where noble's `_compress` keeps `v0`...`v15` in locals and `G1s`/`G2s` return them. Kotlin has no four-value return.
- **Tables are literal.** noble derives `SHA3_PI`, `SHA3_ROTL` and the iota constants at load time and splits `K512` from BigInts. Here they are written out (`words(...)` turns unsigned literals into an `IntArray`).
- **Counters are `Double`,** noble's JS number, split with `fromNumH`/`fromNumL`. This avoids Kotlin/JS's emulated `Long`.
- **Errors** are `IllegalArgumentException`s with noble's messages where cheap. The harness does not compare messages.
- **`null` is absent.** A nullable property cannot tell `null` from `undefined`. The contract puts `null` outside the domain.

## Performance accommodations

- **Loops use `0 until n`, not `step`.** For example, the theta columns are `for (column in 0 until 5)` with `x = 2 * column`. In Kotlin/JS, `until n step k` allocates an `IntProgression` per loop.
- **The chunk list is an `Array`.** It is built with `Array(chunks.size) { ... }`, not `map`, so no `ArrayList` is needed.
