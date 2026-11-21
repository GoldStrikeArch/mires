# lz-string 1.5.0: Rust port (mires)

A hand-written port of `_compress`, `_decompress` and the ten format methods of
`node_modules/lz-string/libs/lz-string.js`, the source the reference wraps, in safe `no_std` Rust
(`#![deny(unsafe_code)]`). LZ codes are written least significant bit first into words whose
first bit is the most significant, with npm's `enlargeIn` / `numBits` growth, end marker and flush.
The output is npm's, but the compressor's dictionary is a different data structure (below).

| File | What it is |
| --- | --- |
| `src/codec.rs` | `compress` and `decompress` over `u16` code units, the `Phrases` dictionary and the `__proto__` automaton |
| `src/lib.rs` | The boundary and the panic handler |

## Boundary

`matrix_run(input)` reads the corpus object through `mires_js` bindings (`Reflect.get`, `length`,
indexed reads). `data` holds UTF-16 code units or bytes and is read element by element into a
`Vec<u16>`, repeated `repeat` times (read as `f64`, ignored for `decompress`). Text never becomes a
Rust `str`, so isolated surrogates and NUL survive. Null or undefined `data` gives an empty array
for every operation. The result is a JS array built with `new Array(length)` and indexed writes, or
`null` where npm returns `null`. Every error is a panic, and the panic handler throws the JS string
`"invalid compressed stream"`.

## The compressor's dictionary

npm's `context_dictionary` maps phrase strings to codes, and every input unit builds the string
`wc = w + c` to look it up. The port keys the same entries by `(code of w, c)` instead: `Phrases`
(`codec.rs:48-80`) is an open-addressing hash table with linear probing whose slots hold a prefix
code, a unit and the entry's code. Prefix 0 holds the single-unit entries (codes start at 3, so 0
is never a real code), and a slot whose `code` is 0 is empty. The table has
`(4n + 8).next_power_of_two()` slots for an `n`-unit input, which holds the at most `2n` entries at
a load of one half or less.

The two are equivalent. Every string in npm's dictionary is the phrase of exactly one code, and
`wc` is in npm's dictionary exactly when `(code(w), c)` is in the table, because each entry npm adds
is some `w` extended by one unit. Codes are assigned in the same order, so the emitted codes are
npm's. The reason is `no_std`: `alloc` has no hash map, and the `(prefix, unit)` key, the usual LZW
trie, needs no phrase strings at all.

`pending` (`codec.rs:96-109`, `113`) is npm's `context_dictionaryToCreate`: a `Vec<u32>` indexed by
code that holds the unit of a single-unit entry until its literal is sent, and `USED` (65,536, a
value no unit has) otherwise. `emit` sends the literal or the code, as npm's two branches do.

## The `__proto__` phrase

npm's `_compress` keeps its dictionary in a plain object, `context_dictionary = {}`. When the phrase
`wc` is the string `__proto__`, `context_dictionary[context_wc] = context_dictSize++` reaches the
inherited `Object.prototype.__proto__` setter, which ignores a number. The phrase never becomes an
own property, but `context_dictSize` still increments, so that code number is spent. Only this name
is affected: npm tests membership with `hasOwnProperty`, so `constructor`, `toString` and the other
`Object.prototype` names are ordinary phrases, and the pending-literal object only ever holds
single units. The corpus cases `keys-*` and `decode-keys-*` exercise it.

The port has no phrase strings to compare with `__proto__`, so a prefix automaton finds the phrase
(`codec.rs:114-116`, `130`, `143-154`). `matched[code]` is `k > 0` when the entry `code` is the
first `k` units of `__proto__`: the single unit `_` gets 1, and a new entry `(prefix, unit)` gets
`matched[prefix] + 1` when `unit` is the next unit of `__proto__`, else 0. When that would reach 9,
the new entry is exactly `__proto__`: the port does not insert it, but still spends its code
(`pending` and `matched` grow by one), as npm does. No longer phrase can start with `__proto__`,
because that entry never exists, so the automaton never passes 9. This reproduces npm; it does not
special-case input.

## Other deviations from the npm source

- **One bit writer.** npm repeats its bit loop for each case: an 8-bit literal, a 16-bit literal, a
  dictionary code, the end marker and the flush. The port has `Writer::bits` (the `count` low bits,
  least significant first) and one `emit` for `context_w`. `Width` is npm's `enlargeIn` and
  `numBits`.
- **Words, then characters.** The writer collects `u16` words of `bitsPerChar` bits, and the
  format maps them at the end: `raw` as is, `utf16` plus 32 with a trailing space, `base64` and
  `uri` through their alphabets (Base64 padded with `=` to a multiple of four), and `bytes` as high
  and low byte. npm maps each word through `getCharFromInt` as it is produced. The flush pads the
  last word with zero bits, or writes a whole zero word when the last word was just completed, as
  npm's flush loop does.
- **Decoder input.** Each format's units become words before decoding: `utf16` subtracts 32
  (wrapping in `u16`, which keeps the low bits npm's mask tests), `bytes` joins big-endian pairs,
  and `base64` and `uri` look each character up in the alphabet (`uri` reads a space as `+`). npm
  reads them lazily through `getNextValue`. A character outside the alphabet becomes 0 (npm's
  `getBaseValue` returns `undefined`, and `undefined & position` is 0), and a read past the end
  gives 0 (npm's `NaN & position`).
- **Decoder dictionary.** A `Vec<Vec<u16>>`. npm stores the numbers 0, 1 and 2 in its first three
  slots; codes 0 to 2 never reach the lookup, so the port leaves them empty, and an empty entry is
  npm's falsy `dictionary[c]`.
- **The undefined `w`.** A stream whose 2-bit header is 3 matches none of npm's `case`s, so `c`
  and `w` stay `undefined`. `dictionary[3]` is then `undefined`, the first output piece joins as an
  empty string, a code equal to `dictSize` throws a TypeError at `w.charAt(0)`, and otherwise the
  next phrase is `"undefined"` followed by the entry's first unit. The port tracks this with
  `defined` and an empty `previous`, and reproduces all three effects; the TypeError is a panic.
- **Results.** `decompress` returns `Option<Vec<u16>>`: `None` where npm returns `null` (an empty
  input, or a code beyond the dictionary), an empty vector where npm returns `""` (the stream runs
  out before its end marker).
- **Odd byte count.** npm's `decompressFromUint8Array` throws a RangeError from
  `new Array(length / 2)`; the port panics at the same point. The harness compares only that both
  threw.
- **`roundtrip` with null data.** The reference adapter throws, because it reads `.length` of the
  decoder's `null`. The port returns an empty array, as it does for null data in every operation.
  `roundtrip` is the benchmark operation, and the corpus never passes it null.

## Module state

None.

## Performance accommodations

None beyond the dictionary design above, which needs no phrase strings.
