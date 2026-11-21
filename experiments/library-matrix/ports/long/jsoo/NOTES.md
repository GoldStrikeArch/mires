# long 5.3.2: js_of_ocaml port

A hand-written OCaml port of the `long@5.3.2` contract subset (`node_modules/long/index.js`).

| File | What it is |
| --- | --- |
| `src/long_ops.ml` | 64-bit operations, parsing, formatting, and command dispatch. Byte-identical to `ports/long/melange/src/long_ops.ml`. |
| `src/main.ml` | The js_of_ocaml JS boundary. |

## Boundary

`run(input)` reads `input.cases` and each command object's fields via `Js.Unsafe.get`, converting strings with `Js.to_string`, arrays with `Js.to_array`, and flags with `Js.to_bool`, and returns `Js.string` of the newline-joined command outputs.

`n` and `r` are read via `Js.Unsafe.get` and arrive in `Long_ops.command` as raw JavaScript numbers without 32-bit truncation:

- `radix` compiles `0 -> 10 | r when r >= 2 && r <= 36` to raw JS number comparisons (`0 === a`, `2 <= a && 36 >= a`), rejecting radices past `2^31`, `2^32`, and `2^53` (and treating out-of-domain `null` as invalid, per `CONTRACT.md`).
- Shift and rotation counts use `n land 63` and `(-n) land 63`, compiling to `n & 63` and `(-n | 0) & 63`, which match JS `ToInt32(n) & 63` across all integer-valued doubles.
- `mix` guards `for i = 0 to n - 1` with `if n > 0 then`. Without this guard, `js_of_ocaml` computes the loop upper bound as `(n - 1) | 0`, which wraps `n = -2147483648` to `2147483647` (hang) and negative values past `-2^31` (such as `-4294967291` and `-9007199254740992`) to small positive iteration counts. The `if n > 0 then` comparison runs on the raw JS number before any `| 0` arithmetic, short-circuiting all `n <= 0` in zero iterations.

Invalid operations, radices, strings, and divisions by zero raise `Invalid_argument`, caught in `command` to emit `"!"`.

## Deviations from the npm source

- **64-bit representation.** Uses OCaml `Int64` rather than `Long` objects or the WebAssembly module.
- **WebAssembly signed division guard.** `divide` explicitly reproduces `long@5.3.2`'s default WebAssembly fast-path overflow check (`I.shift_right a 32 = -2147483648L && b = -1L` returns the dividend), matching `CONTRACT.md`.
- **String parsing and formatting.** `parse` checks empty string and `"NaN"`/`"Infinity"`/`"+Infinity"`/`"-Infinity"` before validating the radix, recurses on leading `'-'` (bounded at 1,000 leading hyphens by `CONTRACT.md`), rejects embedded `'-'`, and consumes 8-character chunks with JS `parseInt` ASCII whitespace, `'+'`, and base-16 `0x`/`0X` prefix rules. `format` writes lowercase digits right-to-left into a 65-byte buffer using `Int64.unsigned_div`.

## Module state

None.

## Performance accommodations

None.
