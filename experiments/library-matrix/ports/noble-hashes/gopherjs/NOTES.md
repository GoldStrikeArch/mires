# @noble/hashes 2.4.0: Go (GopherJS) port

This port is written by hand. It replaces the port that `build/library-matrix/dev-noble-sha2/imperative.py`, `dev-noble-sha3/make_imperative.py` and `dev-noble-blake/dart_go.py` generated. It ports the `sha2`, `sha3`, `blake2` and `hmac` modules of @noble/hashes 2.4.0, as far as `libraries/noble-hashes/CONTRACT.md` covers them, to Go 1.21 compiled by GopherJS 1.21. It follows noble's modules; Go types stand in for its classes.

| File | What it is |
| --- | --- |
| `src/u64.go` | noble's `_u64.ts`: `shrSH`/`shrSL`, `rotrSH`/`rotrSL`, `rotrBH`/`rotrBL`, `rotr32H`/`rotr32L`, `add`, and `add3` to `add5` |
| `src/utils.go` | the `hash` interface, `hashError`, `anumber`, the big- and little-endian word reads and writes, `bytesToHex` |
| `src/sha2.go` | `_md.ts` and `sha2.ts`: `chi`, `maj`, `rotr`, the `sha256` (SHA2_32B) and `sha512` (SHA2_64B) types |
| `src/sha3.go` | `sha3.ts`: `keccakP` and the `keccak` sponge |
| `src/blake2.go` | `blake2.ts` and `_blake.ts`: `bsigma`, `blake2Opts`, `checkBlake2Opts`, `blake2b` with `g1b`/`g2b`, `g1s`/`g2s`, `blake2s` |
| `src/hmac.go` | `hmac.ts`: the `hmac` type |
| `src/main.go` | the boundary: `reference.mjs` in Go |

## Boundary

The input is a `*js.Object`. Fields are read with `Get` and compared with `js.Undefined`. Each JS byte array is copied into a `[]byte` (`Uint8Array.from` in `reference.mjs`). `hasher` is `reference.mjs`'s `hashes` table plus HMAC, so an unknown name fails before any option is read. A one-shot `bytes` message is converted and then hashed. Otherwise the state is created first and the chunks are converted, then fed `repeat` times. This matches the order of `reference.mjs`. There is no one-call `digest(chunks)`: the boundary drives `update` once per chunk. Constructors return Go `error`s; `run` panics with a JS `Error` for any of them, so the call throws where noble throws.

## How it follows noble

- **64-bit words are two `uint32` halves** in every SHA-512 and BLAKE2b variable and array, as in noble's `_u64.ts`. Shifts and rotations are noble's helpers and return one half.
- **The carry formula** is `math/bits.Add32`'s carry out, chained over the low halves in `add` and `add3`...`add5`. Each function returns `(high, low)`. noble sums the unmasked low halves as a JS number and adds `(low / 2 ** 32) | 0` to the high half; the two give the same 64-bit sum. `rotr` is `bits.RotateLeft32(word, -shift)`.
- **noble's `HashMD`** logic is in `sha256` and in `sha512`: `update`, which compresses whole blocks with nothing buffered straight from the input and buffers the rest, and `digest`, which pads with `0x80`, zeros, an extra block when the length field does not fit, and the low 64 bits of the bit length. `sha256` holds noble's `A`...`H` fields, and `process` is noble's.
- **`keccak`** keeps noble's `keccakP` (theta, the single rho-pi cycle, chi with the saved first four words, iota) and its `update`/`finish`/`writeInto` logic on `state32`, including the word fast path.
- **BLAKE2b and BLAKE2s do not share a mix.** `blake2b` has noble's `G1b`/`G2b` on a 32-word work vector of halves (noble's `BBUF`). `blake2s` has noble's `_compress` shape: `v0`...`v15` are locals, and `g1s`/`g2s` return the four mixed words, `v0, v4, v8, v12 = g1s(v0, v4, v8, v12, m[sigma[0]])`. Each type carries noble's `_BLAKE2` `update`, which withholds the last block, and its `digest`. The constructors validate in noble's order: `anumber`, then `checkBlake2Opts`. They then apply the parameter block, salt and personalization, and absorb the zero-padded key.
- **`hmac`** is noble's `_HMAC` over any `hash`. A key longer than a block is first replaced by its digest.

## Deviations from the npm source

None of them changes a result in the contract's domain.

- **Scratch arrays belong to each hash** (the message schedules, BLAKE2's work vector and message words, Keccak's theta columns); noble shares module-level ones.
- **No zeroisation, no lifecycle flags, no `clone`.** Each state is digested once per call and dropped.
- **Keccak's byte path shifts each byte into its word.** noble XORs through a byte view of `state32`. The word path does not depend on alignment.
- **BLAKE2's whole-block path has no alignment test.**
- **Counters are `uint64`.** noble's are JS numbers.
- **SHA-512 state is a `[16]uint32` of halves** where noble has sixteen fields.
- **Tables are literal**, in one `var ( ... )` block per file. Go has no constant arrays. These blocks are the module-state lint's findings, and every table in them is read-only. noble derives `SHA3_PI`, `SHA3_ROTL` and the iota constants at load time, and splits `K512` from BigInts.
- **Error messages** are shorter than noble's. The harness does not compare messages.
- **`null`**, which the contract puts outside the domain, is not `js.Undefined`. A null byte array therefore fails to convert and throws, as npm's `Uint8Array.from(null)` does. A null number reads as 0, which BLAKE2 rejects and which runs no updates as a `repeat`. Both match npm.

## Performance and size accommodations

- **No shared `HashMD` or `_BLAKE2` base.** Each of `sha256`, `sha512`, `blake2b` and `blake2s` has its own `update` and `digest`, as Go's own `crypto/sha256` and `crypto/sha512` do. A shared base would call `process` through an interface or a function value. GopherJS compiles such a call, and every function that makes it, into a resumable state machine (README, "Per-language notes"). That would put every block's compression behind one.
- **`hashError` instead of the `errors` package.** `errors.New` pulls in `internal/reflectlite` and `internal/abi`, about 42 KB of emit.
