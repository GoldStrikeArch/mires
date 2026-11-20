# @noble/hashes 2.4.0: ReScript port

This port is written by hand. It replaces the port that `build/library-matrix/dev-noble-sha2/imperative.py`, `dev-noble-sha3/make_imperative.py` and `dev-noble-blake/ml.py` generated. It ports the `sha2`, `sha3`, `blake2` and `hmac` modules of @noble/hashes 2.4.0, as far as `libraries/noble-hashes/CONTRACT.md` covers them, to ReScript 12.3. It follows noble's modules: each class becomes a mutable record with module functions, and a `Hash.t` record of closures stands in for the object.

| File | What it is |
| --- | --- |
| `src/U64.res` | noble's `_u64.ts`: `shrSH`/`shrSL`, `rotrSH`/`rotrSL`, `rotrBH`/`rotrBL`, `rotr32H`/`rotr32L`, `fromNumH`/`fromNumL`, `add`, and `add3` to `add5` |
| `src/Hash.res` | `Hash.t`: block and digest lengths, `update` and `digest` |
| `src/Utils.res` | `anumber`, `rotr`, the big- and little-endian word reads and writes, `bytesToHex` |
| `src/Md.res` | `_md.ts`: `chi`, `maj` and HashMD (`make`, `update`, `digest`, `toHash`) |
| `src/Sha2.res` | `sha2.ts`: SHA-256 (SHA2_32B) and SHA-512 (SHA2_64B) |
| `src/Sha3.res` | `sha3.ts`: `keccakP` and the Keccak sponge |
| `src/Blake2.res` | `blake2.ts` and `_blake.ts`: `bsigma`, `opts`, `checkBlake2Opts`, the `_BLAKE2` base, `blake2b` with `g1b`/`g2b`, `blake2s` with `g1s`/`g2s` |
| `src/Hmac.res` | `hmac.ts` |
| `src/Index.res`, `src/Index.resi` | the boundary: `reference.mjs` in ReScript; `run` is the only export |

## Boundary

The input record is the corpus object itself, with optional fields, so reading it converts nothing. Its byte arrays are `array<int>` and are read in place, never written. `reference.mjs` copies each one with `Uint8Array.from`. `hasher` is `reference.mjs`'s `hashes` table plus HMAC, so an unknown name throws before any option is read. The state is created, then fed the one-shot `bytes` once or the `chunks` `repeat` times. There is no one-call `digest(chunks)`: the boundary drives `update` once per chunk. The digest returns as a JS string.

## How it follows noble

- **64-bit words are two 32-bit `int` halves** in every SHA-512 and BLAKE2b variable and array, as in noble's `_u64.ts`. Shifts and rotations are noble's helpers and return one half.
- **The carry formula.** `add` and `add3`...`add5` return both halves as a tuple. Each carry out of the low halves is an unsigned comparison of the wrapped sum with the addend, written as a signed comparison after flipping both sign bits, `(sum ^^^ -2147483648) < (addend ^^^ -2147483648)`. noble sums the unmasked low halves as a JS number and adds `(low / 2 ** 32) | 0` to the high half. ReScript's `int` is 32-bit, and `Int.shiftRightUnsigned(x, 0)` folds away, so the unsigned value cannot be held in an `int`. The two give the same 64-bit sum.
- **Bitwise operators** are ReScript 12's own `&&&`, `|||`, `^^^`, `~~~`, `<<` and `>>>`. They compile to JS operators, as noble's do. The generated port bound the `%andint`-style primitives directly, which is the review finding fixed here.
- **HashMD** (`Md.res`) keeps noble's `update` and `digestInto`. Whole blocks with nothing buffered compress straight from the input. Padding is `0x80` and zeros, with an extra block when the length field does not fit. The length field is the low 64 bits of the bit length. `process` and `get` are the record's closures, where noble has abstract methods. SHA-256's state record has noble's `A`...`H` fields, and both `process` functions are noble's.
- **Keccak** keeps noble's `keccakP` (theta, the single rho-pi cycle, chi with the saved first four words, iota) and its `update`/`finish`/`writeInto` logic on `state32`.
- **BLAKE2b and BLAKE2s do not share a mix.** `blake2b` has noble's `G1b`/`G2b` on a 32-word work vector of halves (noble's `BBUF`); `blake2s` has `G1s`/`G2s` on 32-bit words. Both use the `_BLAKE2` base, whose `update` withholds the last block. The constructors validate in noble's order: `anumber`, then `checkBlake2Opts`. They then apply the parameter block, salt and personalization, and absorb the zero-padded key.
- **HMAC** is noble's `_HMAC` over any `Hash.t`. A key longer than a block is first replaced by its digest.

## Deviations from the npm source

None of them changes a result in the contract's domain.

- **Byte arrays are not copied** at the boundary (see Boundary).
- **Scratch arrays belong to each hash** (the message schedules, BLAKE2's work vector and message words, Keccak's theta columns); noble shares module-level ones.
- **No zeroisation, no lifecycle flags, no `clone`.** Each state is digested once per call and dropped.
- **Keccak's byte path shifts each byte into its word.** noble XORs through a byte view of `state32`. The word path does not depend on alignment.
- **BLAKE2's whole-block path has no alignment test.** The base passes the byte counter to `compress`, where noble's `compress` reads `this.length`.
- **State as arrays.** SHA-512's state is an array of halves and BLAKE2b's eight words an array of halves, where noble has sixteen fields. BLAKE2s works on a 16-word array by index, where noble's `_compress` keeps `v0`...`v15` in locals and `G1s`/`G2s` return them.
- **Tables are literal.** noble derives `SHA3_PI`, `SHA3_ROTL` and the iota constants at load time, and splits `K512` from BigInts.
- **Counters are `float`,** noble's JS number, split with `fromNumH`/`fromNumL`.
- **Errors** are JS `Error`, `TypeError` and `RangeError`s with noble's messages. The harness does not compare messages.
- **`null`**, which the contract puts outside the domain, is a present value for the optional record fields. BLAKE2 rejects a null `dkLen`, `key`, `salt` or `personalization`. The other hashes ignore a null option, where npm's `Uint8Array.from(null)` throws.

## Performance accommodations

None.
