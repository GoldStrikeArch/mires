# @noble/hashes 2.4.0: Melange port

This port is written by hand. It replaces the port that `build/library-matrix/dev-noble-sha2/ml.py`, `dev-noble-sha3/ml.py` and `dev-noble-blake/ml.py` generated. It ports the `sha2`, `sha3`, `blake2` and `hmac` modules of @noble/hashes 2.4.0, as far as `libraries/noble-hashes/CONTRACT.md` covers them, to OCaml. It follows noble's modules: each class becomes a mutable record with module functions, and a `Hash.t` record of closures stands in for the object.

**Shared source.** The Melange and js_of_ocaml ports share one OCaml source: `u32.ml`, `u64.ml`, `hash.ml`, `md.ml`, `sha2.ml`, `sha3.ml`, `blake2.ml`, `hmac.ml` and `hashes.ml` are byte-identical in `ports/noble-hashes/melange/src/` and `ports/noble-hashes/jsoo/src/`. Only the boundary module differs: Melange's `index.ml`, js_of_ocaml's `main.ml`. Change the shared files in both ports together.

| File | What it is |
| --- | --- |
| `u32.ml` | `Int32` with `+`, `land`, `lor`, `lxor`, `lsl`, `lsr` and `lnot` under their `int` names, for local opens such as `U32.(a lxor b + c)`; noble's `rotr` |
| `u64.ml` | noble's `_u64.ts`: `shr_sh`/`shr_sl`, `rotr_sh`/`rotr_sl`, `rotr_bh`/`rotr_bl`, `rotr32_h`/`rotr32_l`, `from_num_h`/`from_num_l`, `add`, and `add3` to `add5` |
| `hash.ml` | `Hash.t` (block and digest lengths, `update`, `digest`), `anumber`, the big- and little-endian word reads and writes, `bytes_to_hex` |
| `md.ml` | `_md.ts`: `chi`, `maj` and HashMD (`make`, `update`, `digest`, `to_hash`) |
| `sha2.ml` | `sha2.ts`: SHA-256 (SHA2_32B) and SHA-512 (SHA2_64B) |
| `sha3.ml` | `sha3.ts`: `keccak_p` and the Keccak sponge |
| `blake2.ml` | `blake2.ts` and `_blake.ts`: `bsigma`, `opts`, `check_blake2_opts`, the `_BLAKE2` base, `blake2b` with `g1b`/`g2b`, `blake2s` with `g1s`/`g2s` |
| `hmac.ml` | `hmac.ts` |
| `hashes.ml` | `reference.mjs` on OCaml values: the `hashes` table plus HMAC (`hasher`) and `run` |
| `index.ml` | the Melange boundary |

## Boundary

The input is a `< ... > Js.t` object read with `##`; absent fields are `Js.undefined`. A JS array of numbers is an OCaml `int array` and a JS number an OCaml `int` or `float`, so reading it converts nothing. The byte arrays are read in place and never written, where `reference.mjs` copies each one with `Uint8Array.from`. `run` returns the OCaml string, which is the JS string.

## How it follows noble

- **64-bit words are two `int32` halves** in every SHA-512 and BLAKE2b variable and array, as in noble's `_u64.ts`. Shifts and rotations are noble's helpers and return one half. `Int32` keeps the arithmetic 32-bit on every OCaml target: native `int` is 63-bit, while Melange's and js_of_ocaml's `int` is 32-bit.
- **The carry formula.** `add` and `add3`...`add5` return both halves as a pair. Each carry out of the low halves is `Int32.unsigned_compare sum addend < 0`, taken after every wrapped 32-bit addition. noble sums the unmasked low halves as a JS number and adds `(low / 2 ** 32) | 0` to the high half. The two give the same 64-bit sum. noble splits each multi-term sum into `addNL` and `addNH`; `addN` here computes both halves at once.
- **HashMD** (`md.ml`) keeps noble's `update` and `digestInto`. Whole blocks with nothing buffered compress straight from the input. Padding is `0x80` and zeros, with an extra block when the length field does not fit. The length field is the low 64 bits of the bit length. `process` and `get` are the record's closures, where noble has abstract methods. Both `process` functions are noble's, with its working variables as `ref`s.
- **Keccak** keeps noble's `keccakP` (theta, the single rho-pi cycle, chi with the saved first four words, iota) and its `update`/`finish`/`writeInto` logic on `state32`.
- **BLAKE2b and BLAKE2s do not share a mix.** `blake2b` has noble's `G1b`/`G2b` on a 32-word work vector of halves (noble's `BBUF`); `blake2s` has `G1s`/`G2s` on 32-bit words. Both use the `_BLAKE2` base, whose `update` withholds the last block. The constructors validate in noble's order: `anumber`, then `checkBlake2Opts`. They then apply the parameter block, salt and personalization, and absorb the zero-padded key.
- **HMAC** is noble's `_HMAC` over any `Hash.t`. A key longer than a block is first replaced by its digest.
- **`Hashes.run`** creates the state, then feeds the one-shot message once or the chunks `repeat` times. There is no one-call `digest(chunks)`: the boundary drives `update` once per chunk, as `reference.mjs` does.

## Deviations from the npm source

None of them changes a result in the contract's domain.

- **Scratch arrays belong to each hash** (the message schedules, BLAKE2's work vector and message words, Keccak's theta columns); noble shares module-level ones.
- **No zeroisation, no lifecycle flags, no `clone`.** Each state is digested once per call and dropped.
- **Keccak's byte path shifts each byte into its word.** noble XORs through a byte view of `state32`. The word path does not depend on alignment.
- **BLAKE2's whole-block path has no alignment test.** The base passes the byte counter to `compress`, where noble's `compress` reads `this.length`.
- **State as arrays.** SHA-512's state is an `int32 array` of halves and BLAKE2b's eight words an array of halves, where noble has sixteen fields. BLAKE2s works on a 16-word array by index, where noble's `_compress` keeps `v0`...`v15` in locals and `G1s`/`G2s` return them.
- **Tables are literal.** noble derives `SHA3_PI`, `SHA3_ROTL` and the iota constants at load time, and splits `K512` from BigInts.
- **Counters are `float`,** noble's JS number, split with `from_num_h`/`from_num_l`. `int` is 32-bit under both compilers.
- **Errors** are `Invalid_argument` with noble's messages. The harness does not compare messages.
- **Byte arrays are not copied** (see Boundary).
- **`null`**, which the contract puts outside the domain, is a present value because only `undefined` is absent. BLAKE2 rejects a null `dkLen`, `key`, `salt` or `personalization`. The other hashes ignore a null option, where npm's `Uint8Array.from(null)` throws.

## Performance accommodations

None.
