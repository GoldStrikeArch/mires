# @noble/hashes 2.4.0: Dart port

This port is written by hand. It replaces the port that `build/library-matrix/dev-noble-sha2/imperative.py`, `dev-noble-sha3/make_imperative.py` and `dev-noble-blake/dart_go.py` generated. It ports the `sha2`, `sha3`, `blake2` and `hmac` modules of @noble/hashes 2.4.0, as far as `libraries/noble-hashes/CONTRACT.md` covers them, to Dart 3 compiled by dart2js. It follows noble's modules and classes.

| File | What it is |
| --- | --- |
| `src/u64.dart` | noble's `_u64.ts`: `shrSH`/`shrSL`, `rotrSH`/`rotrSL`, `rotrBH`/`rotrBL`, `rotr32H`/`rotr32L`, `fromNumH`/`fromNumL`, `add`, and `add3L`/`add3H`, `add4L`/`add4H`, `add5L`/`add5H` |
| `src/utils.dart` | the `Hash` interface, `anumber`, `rotr`, `bytesToHex` |
| `src/md.dart` | `_md.ts`: `chi`, `maj` and the abstract `HashMD` |
| `src/sha2.dart` | `sha2.ts`: `Sha256` (SHA2_32B) and `Sha512` (SHA2_64B) |
| `src/sha3.dart` | `sha3.ts`: `keccakP` and the `Keccak` sponge |
| `src/blake2.dart` | `blake2.ts` and `_blake.ts`: `Blake2Opts`, `_checkBlake2Opts`, the abstract `Blake2` (noble's `_BLAKE2`), `Blake2b`, `_g1s`/`_g2s`, `Blake2s` |
| `src/hmac.dart` | `hmac.ts`: `Hmac` |
| `src/main.dart` | the boundary: `reference.mjs` in Dart; `main()` publishes `run` |

## Boundary

The input is read through a `dart:js_interop` extension type with nullable getters. Each `JSArray<JSNumber>` is copied into a `Uint8List` (`Uint8Array.from` in `reference.mjs`). `_hasher` is `reference.mjs`'s `hashes` table plus HMAC, so an unknown name throws before any option is read. A one-shot `bytes` message is converted and then hashed. Otherwise the state is created first and the chunks are converted, then fed `repeat` times. This matches the order of `reference.mjs`. There is no one-call `digest(chunks)`: the boundary drives `update` once per chunk. The digest returns as a JS string.

## How it follows noble

- **64-bit words are two 32-bit halves** in every SHA-512 and BLAKE2b variable and `Uint32List`, as in noble's `_u64.ts`. A half is an `int` in `0..0xffffffff`, the value a `Uint32List` stores. Results that a shift, `~` or a sum can widen are masked with `mask32`, so the code means the same on the web and on the Dart VM.
- **The carry formula is noble's.** The `addNL` functions sum the low halves unmasked; an `int` holds the sum exactly on both platforms. `addNH` adds `low ~/ 2^32` to the high half, which is noble's `(low / 2 ** 32) | 0`. `add` returns noble's `{h, l}` as the record `({int h, int l})`, and the call sites destructure it as noble does: `(h: eh, l: el) = add(dh, dl, t1h, t1l)`.
- **`HashMD`** keeps noble's `update` and `digestInto`, including the `ByteData` (DataView) reads and writes. Whole blocks with nothing buffered compress straight from a view of the input. Padding is `0x80` and zeros, with an extra block when the length field does not fit. The length field is the low 64 bits of the bit length. `Sha256` holds noble's `A`...`H` fields, and `process` is noble's.
- **`Keccak`** keeps noble's `keccakP` (theta, the single rho-pi cycle, chi with the saved first four words, iota) and its `update`/`finish`/`writeInto` logic on `state32`, including the little-endian word fast path.
- **BLAKE2b and BLAKE2s do not share a mix.** BLAKE2b has noble's `G1b`/`G2b`, using `add3L`/`add3H` and `add`, on a 32-word work vector of halves (noble's `BBUF`). BLAKE2s has noble's `_compress` shape: `v0`...`v15` are locals, and `_g1s`/`_g2s` return the four mixed words as a record, `(v0, v4, v8, v12) = _g1s(...)`. Both hashes sit on `Blake2`, noble's `_BLAKE2`. Its `update` withholds the last block. The constructors validate in noble's order: `anumber`, then `checkBlake2Opts`. They then apply the parameter block, salt and personalization, and absorb the zero-padded key.
- **`Hmac`** is noble's `_HMAC` over any `Hash`. A key longer than a block is first replaced by its digest.

## Deviations from the npm source

None of them changes a result in the contract's domain.

- **Scratch lists belong to each hash** (the message schedules, BLAKE2's work vector and message words, Keccak's theta columns); noble shares module-level ones.
- **No zeroisation, no lifecycle flags, no `clone`.** Each state is digested once per call and dropped.
- **Keccak's byte path shifts each byte into its word.** noble XORs through a byte view of `state32`. Little-endian words are read with `ByteData`, so no host-endianness swap is needed, and the word path does not depend on alignment.
- **BLAKE2's whole-block path has no alignment test.** It reads little-endian words through `ByteData`.
- **SHA-512 and BLAKE2 state are `Uint32List`s.** noble has sixteen (SHA-512, BLAKE2b) or eight (BLAKE2s) fields.
- **Tables are literal `const` lists.** noble derives `SHA3_PI`, `SHA3_ROTL` and the iota constants at load time, and splits `K512` from BigInts.
- **Errors** are `ArgumentError`s with noble's messages. The harness does not compare messages.
- **`null` is absent.** The nullable getters cannot tell `null` from `undefined`. The contract puts `null` outside the domain.

## Performance accommodations

None.
