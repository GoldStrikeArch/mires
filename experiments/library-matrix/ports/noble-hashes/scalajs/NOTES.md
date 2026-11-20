# @noble/hashes 2.4.0: Scala.js port

This port is written by hand. It replaces the port that `build/library-matrix/dev-noble-sha2/imperative.py`, `dev-noble-sha3/make_imperative.py` and `dev-noble-blake/jvm.py` generated. It ports the `sha2`, `sha3`, `blake2` and `hmac` modules of @noble/hashes 2.4.0, as far as `libraries/noble-hashes/CONTRACT.md` covers them, to Scala 3.8 on Scala.js 1.22. It follows noble's modules and classes.

| File | What it is |
| --- | --- |
| `src/U64.scala` | `object U64`, noble's `_u64.ts`: `shrSH`/`shrSL`, `rotrSH`/`rotrSL`, `rotrBH`/`rotrBL`, `rotr32H`/`rotr32L`, `fromNumH`/`fromNumL`, `add`, and `add3` to `add5` |
| `src/Utils.scala` | the `Hash` trait, and `object Utils`: `anumber`, the big- and little-endian word reads and writes, `bytesToHex` |
| `src/Md.scala` | `_md.ts`: `chi`, `maj` and the abstract `HashMD` |
| `src/Sha2.scala` | `sha2.ts`: the constants, `Sha256` (SHA2_32B) and `Sha512` (SHA2_64B) |
| `src/Sha3.scala` | `sha3.ts`: the tables, `keccakP` and the `Keccak` sponge |
| `src/Blake2.scala` | `blake2.ts` and `_blake.ts`: `BSIGMA`, `checkBlake2Opts`, `Blake2Opts`, the abstract `Blake2Base` (noble's `_BLAKE2`), `Blake2b`, `Blake2s` |
| `src/Hmac.scala` | `hmac.ts`: `Hmac` |
| `src/Main.scala` | the `HashInput` facade and the boundary: `reference.mjs` in Scala |

## Boundary

The input is read in place through a `@js.native` trait with `js.UndefOr` fields. Each `js.Array[Int]` is copied into an `Array[Byte]` (`Uint8Array.from` in `reference.mjs`). `hasher` is `reference.mjs`'s `hashes` table plus HMAC, so an unknown name throws before any option is read. A one-shot `bytes` message is converted and then hashed. Otherwise the state is created first and the chunks are converted, then fed `repeat` times. This matches the order of `reference.mjs`. There is no one-call `digest(chunks)`: the boundary drives `update` once per chunk. `run` is exported with `@JSExportTopLevel("run")` and returns a `String`.

## How it follows noble

- **64-bit words are two 32-bit `Int` halves** in every SHA-512 and BLAKE2b variable and array, as in noble's `_u64.ts`. Shifts and rotations are noble's helpers and return one half.
- **The carry formula.** `add` and `add3`...`add5` return both halves as a tuple. Each carry out of the low halves is `Integer.compareUnsigned(sum, addend) < 0`, taken after every wrapped 32-bit addition. noble instead sums the unmasked low halves as a JS number and adds `(low / 2 ** 32) | 0` to the high half. The two give the same 64-bit sum. noble splits each multi-term sum into `addNL` and `addNH`; `addN` here computes both halves at once.
- **`HashMD`** keeps noble's `update` and `digestInto`: whole blocks with nothing buffered compress straight from the input; padding is `0x80`, zeros, an extra block when the length field does not fit, and the low 64 bits of the bit length. `Sha256` holds noble's `A`...`H` fields, `Sha512` its `Ah`, `Al`...`Hh`, `Hl` fields, and `process` is noble's.
- **`Keccak`** keeps noble's `keccakP` (theta, the single rho-pi cycle, chi with the saved first four words, iota) and its `update`/`finish`/`writeInto` logic on `state32`.
- **BLAKE2b and BLAKE2s do not share a mix.** BLAKE2b has noble's `G1b`/`G2b` on a 32-word work vector of halves (noble's `BBUF`); BLAKE2s has `G1s`/`G2s` on 32-bit words. Both sit on `Blake2Base`, noble's `_BLAKE2`. Its `update` withholds the last block, because the final block is compressed with a flag. The constructors validate in noble's order: `anumber`, then `checkBlake2Opts`. They then apply the parameter block, salt and personalization, and absorb the zero-padded key.
- **`Hmac`** is noble's `_HMAC` over any `Hash`. A key longer than a block is first replaced by its digest.

## Deviations from the npm source

None of them changes a result in the contract's domain.

- **Scratch arrays belong to each hash** (the message schedules, BLAKE2's work vector, Keccak's theta columns); noble shares module-level ones.
- **No zeroisation, no lifecycle flags, no `clone`.** Each state is digested once per call and dropped.
- **Keccak absorbs into 32-bit words directly.** noble XORs bytes through a byte view of `state32`. Here each byte is shifted into its word. Every block absorbed with nothing buffered takes the word path, where noble also requires 4-byte alignment.
- **BLAKE2's whole-block path has no alignment test.** noble's zero-copy path also requires a 4-byte-aligned `byteOffset`.
- **BLAKE2 state as arrays.** BLAKE2b's eight words are an `Array[Int]` of halves where noble has sixteen fields. BLAKE2s works on a 16-word array by index, where noble's `_compress` keeps `v0`...`v15` in locals and `G1s`/`G2s` return four-field objects.
- **Tables are literal.** noble derives `SHA3_PI`, `SHA3_ROTL` and the iota constants at load time, and splits `K512` from BigInts. Here they are written out.
- **Counters are `Double`,** noble's JS number, split with `fromNumH`/`fromNumL`. This avoids Scala.js's emulated `Long`.
- **Errors** are `IllegalArgumentException`s from `require`, with noble's messages. The harness does not compare messages.
- **`null`**, which the contract puts outside the domain, counts as present for `js.UndefOr`. A null byte array then fails to convert and throws, like npm's `Uint8Array.from(null)`. A null number unboxes to 0, which BLAKE2 rejects as a digest length and which runs no updates as a `repeat`. Both match npm.

## Module-state lint

The lint flags 30 lines, all of them `private var` and `protected var` fields of the hash classes. These are noble's instance fields: `A`...`H`, `Ah`...`Hl`, `pos`, `posOut` and `length`. Its two-space pattern cannot tell them from an `object`'s members. No `object` holds mutable state.

## Performance accommodations

None.
