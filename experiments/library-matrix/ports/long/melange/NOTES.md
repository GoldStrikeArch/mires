# long 5.3.2: Melange port

A hand-written OCaml port of the `long@5.3.2` contract subset (`node_modules/long/index.js`).

| File | What it is |
| --- | --- |
| `src/long_ops.ml` | 64-bit operations, parsing, formatting, and command dispatch. Byte-identical to `ports/long/jsoo/src/long_ops.ml`. |
| `src/index.ml` | The Melange JS boundary. |

## Boundary

`run(input)` reads `input.cases` and each command record's fields directly via Melange JS object accessors (`c##op`, `c##a`, `c##b`, `c##u`, `c##v`, `c##n`, `c##r`, `c##s`) and joins command outputs with `"\n"`.

`n` and `r` are typed as OCaml `int` at the Melange boundary and arrive as raw JavaScript numbers without 32-bit truncation:

- `radix` compares `r = 0` and `2 <= r && r <= 36` directly against the JS number, rejecting radices past `2^31`, `2^32`, and `2^53` (and treating out-of-domain `null` as invalid, per `CONTRACT.md`).
- Shift and rotation counts use `n land 63` and `(-n) land 63`, which lower to JS `& 63` (`ToInt32(n) & 63`).
- `mix` guards `for i = 0 to n - 1` with `if n > 0 then` so that negative counts (including `-2^31`, `-2^32 + 5`, `-2^53`, and `-Number.MAX_VALUE`) short-circuit before `n - 1` is computed; Melange lowers `for i = 0 to n - 1` to `i < n`, and the explicit guard keeps the shared `long_ops.ml` safe under `js_of_ocaml`'s 32-bit `n - 1 | 0` upper-bound lowering as well.

Invalid operations, radices, strings, and divisions by zero raise `Invalid_argument`, caught in `command` to emit `"!"`.

## Deviations from the npm source

- **64-bit representation.** Uses OCaml `Int64` (two 32-bit words in Melange's runtime) rather than `Long` objects or the WebAssembly module.
- **WebAssembly signed division guard.** `divide` explicitly reproduces `long@5.3.2`'s default WebAssembly fast-path overflow check (`I.shift_right a 32 = -2147483648L && b = -1L` returns the dividend), matching `CONTRACT.md`.
- **String parsing and formatting.** `parse` checks empty string and `"NaN"`/`"Infinity"`/`"+Infinity"`/`"-Infinity"` before validating the radix, recurses on leading `'-'` (bounded at 1,000 leading hyphens by `CONTRACT.md`), rejects embedded `'-'`, and consumes 8-character chunks with JS `parseInt` ASCII whitespace, `'+'`, and base-16 `0x`/`0X` prefix rules. `format` writes lowercase digits right-to-left into a 65-byte buffer using `Int64.unsigned_div`.

## Module state

None.

## Performance accommodations

None.
