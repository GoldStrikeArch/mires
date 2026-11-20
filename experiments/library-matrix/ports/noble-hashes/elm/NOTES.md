# @noble/hashes 2.4.0: Elm port

This port is written by hand. It replaces the port that `build/library-matrix/dev-noble-sha2/elm.py`, `dev-noble-sha3/elm.py` and `dev-noble-blake/elm.py` generated. It ports the `sha2`, `sha3`, `blake2` and `hmac` modules of @noble/hashes 2.4.0, as far as `libraries/noble-hashes/CONTRACT.md` covers them, to Elm 0.19. Elm has no mutation, so each of noble's classes becomes an immutable record and its methods become functions from one state to the next. The module split, the helpers and the compression steps are noble's.

| File | What it is |
| --- | --- |
| `src/U64.elm` | noble's `_u64.ts`: `Word` (`{h, l}`), `shrSH`/`shrSL`, `rotrSH`/`rotrSL`, `rotrBH`/`rotrBL`, `rotr32H`/`rotr32L`, `fromNumH`/`fromNumL`, `add`, `add3L`/`add3H`, `add4L`/`add4H`, `add5L`/`add5H` |
| `src/Words.elm` | `rotr`, and the byte-to-word conversions standing in for noble's DataView and Uint32Array views |
| `src/Md.elm` | `_md.ts`: `chi`, `maj` and HashMD (`Md s`, `make`, `update`, `digest`) |
| `src/Sha2.elm` | `sha2.ts`: SHA-256 (SHA2_32B) and SHA-512 (SHA2_64B) |
| `src/Sha3.elm` | `sha3.ts`: `keccakP` with theta, rho-pi, chi and iota, and the Keccak sponge |
| `src/Blake2.elm` | `_blake.ts`'s BSIGMA, `Options`, and the `_BLAKE2` base (`Blake2 s`, `make`, `update`, `digest`) with `checkBlake2Opts` |
| `src/Blake2b.elm` | `_BLAKE2b` with `g1b`/`g2b` |
| `src/Blake2s.elm` | `_BLAKE2s` with `g1s`/`g2s` |
| `src/Hashes.elm` | the `Hash` type over the hashes above, `create` (the `hashes` table plus HMAC), `update`, `digest`, and `hmac.ts`'s `_HMAC` |
| `src/Main.elm` | the ports and the decoding: `reference.mjs` in Elm |
| `js/index.js` | the JS host: sends the input through the `request` port and returns the `response` |

## Boundary

The host (`js/index.js`) starts the worker on the first call, sends each input through `request`, and returns `ok` from the synchronous `response` or throws `error`. `Main.run` decodes the input with `Json.Decode`. It reads the algorithm first, so an unknown name fails before any option, then the options, then the message. Byte arrays become `List Int`. An optional field is `Nothing` only when absent. When present, it must decode, so a null or wrongly typed `key`, `salt`, `personalization` or `repeat` is an error. `dkLen` is kept as `reference.mjs` passes it, whatever its type, and only BLAKE2 requires a number. A null or non-numeric `dkLen` therefore fails on BLAKE2 and is ignored by the other hashes, as in npm. The state is created, then fed the one-shot `bytes` once or the `chunks` `repeat` times. There is no one-call `digest(chunks)`: `update` runs once per chunk.

## How it follows noble

- **64-bit words are two 32-bit halves.** They are separate `Int` fields in the SHA-512 state (noble's `Ah`, `Al`...`Hh`, `Hl`) and `Word` records in BLAKE2b, as in noble's `_u64.ts`. Shifts and rotations are noble's helpers and return one half.
- **The carry formula is noble's.** Elm's `Int` is a JS number. `addNL` sums the unmasked low halves (`shiftRightZfBy 0`, noble's `>>> 0`), and `addNH` adds `low // 2^32` to the high half. Elm's `//` is noble's `(low / 2 ** 32) | 0`. Sums are reduced with `or 0`, noble's `| 0`.
- **HashMD** (`Md s`) holds noble's fields and a compression state `s`. `process` and `get` are record fields, where noble has abstract methods. `digest` pads with `0x80`, zeros, an extra block when the length field does not fit, and the low 64 bits of the bit length. SHA-256's state is noble's `A`...`H`, and its schedule and rounds are noble's: an `Array` for `W` and tail-recursive loops over `i`.
- **Keccak** keeps noble's step order: theta (each lane takes the parity of the column before it and the rotated parity of the column after it), rho and pi, chi, iota. Between permutations the state is noble's 25 lanes in noble's order (x + 5y), each lane its first (low) and second (high) word. `keccakP` turns them into a record whose fields follow the Keccak team's naming (`aba` ... `asu`), so each step maps the whole state to the next. Rho and pi are written out lane by lane in the order of noble's single cycle, with noble's rotation for each step.
- **BLAKE2b and BLAKE2s do not share a mix.** `Blake2b` has noble's `G1b`/`G2b` on 64-bit words with `add3L`/`add3H`; `Blake2s` has `G1s`/`G2s` on 32-bit words. Each takes and returns the four words it mixes (noble's `G1s` returns `{a, b, c, d}`). A round mixes the four columns, then the four diagonals, with noble's message word order. Both use the `_BLAKE2` base, whose `update` withholds the last block. The constructors validate in noble's order: `anumber`, then `checkBlake2Opts`. They then apply the parameter block, salt and personalization, and absorb the zero-padded key.
- **HMAC** is noble's `_HMAC` over any `Hash`. A key longer than a block is first replaced by its digest.

## Deviations from the npm source

None of them changes a result in the contract's domain.

- **Buffering is on lists.** noble fills a block buffer and compresses whole blocks in place. Here `update` prepends the buffered bytes to the new ones, compresses each whole block of the result in order, and keeps the rest as the buffer. BLAKE2 keeps the last block back. The blocks, their order and their counters are noble's. BLAKE2's `compressed` counts compressed bytes, where noble's `length` also counts buffered ones; the counter passed to `compress` is the same.
- **Keccak buffers too.** noble XORs each byte into the state as it arrives. Here a partial block waits, as in HashMD, and is XORed into the lanes when it completes. The final block carries the suffix and `0x80` (noble's `finish`, including its extra permutation for a suffix with bit 7 set). The permutations happen at the same points.
- **Keccak squeezes one block.** noble's `writeInto` loops, permuting again whenever the output passes the rate. Both digests here, 32 bytes, are shorter than the 136-byte rate.
- **HMAC builds the outer pad from the key directly** (`key ^ 0x5c`). noble XORs its inner pad again with `0x36 ^ 0x5c`; the bytes are the same.
- **The message schedule is an `Array`** built with `push`. SHA-512's is two arrays, `wh` and `wl`, as in noble.
- **No zeroisation, no lifecycle flags, no `clone`.**
- **Tables are literal.** noble derives `SHA3_PI`, `SHA3_ROTL` and the iota constants at load time, and splits `K512` from BigInts.
- **Errors** are `Err` strings that the host throws as `TypeError`s. The harness does not compare messages.
- **A null `repeat`**, outside the contract's domain, is an error. npm runs no updates for it.
- **Module state.** `js/index.js` keeps the Elm worker (`app`) and the last response (`reply`) across calls; these are the module-state lint's two findings. The worker's model is `()`, so nothing from one call reaches the next.

## Performance accommodations

None. Every loop over input bytes, blocks, chunks or rounds is tail-recursive or a core `List` function that does not grow the stack. The few plain recursions (`xorLanes`, `toLanes`, `Blake2b.toWords`) are bounded by one block. The 2 MiB one-shot message and the 200,000-entry chunks array in the corpus therefore run in constant stack.
