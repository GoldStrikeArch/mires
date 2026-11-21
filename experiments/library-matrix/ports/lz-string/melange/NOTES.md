# lz-string 1.5.0: Melange port

A hand-written port of `_compress`, `_decompress` and the ten format methods of
`node_modules/lz-string/libs/lz-string.js`, the source the reference wraps. LZ codes are written
least significant bit first into words whose first bit is the most significant, with npm's
`enlargeIn` / `numBits` growth, end marker and flush.

| File | What it is |
| --- | --- |
| `src/codec.ml` | The codec and the operation dispatch. Byte-identical to `ports/lz-string/jsoo/src/codec.ml`. |
| `src/index.ml` | The Melange boundary |

## Boundary

`run(input)` reads the corpus object through `[@@mel.get_index]` field access. `data` is
`Js.Nullable.t` of an `int array` of UTF-16 code units or bytes, and an absent `repeat` means 1.
Phrases are OCaml strings with exactly two bytes per UTF-16 unit, high byte first (`symbol`,
`unit_at`, `text`, `units`), so isolated surrogates and NUL survive without any UTF-8 conversion.
The result is an `int array` (a JS array of numbers), or `Js.null` where npm returns `null`. Null
`data` gives an empty array for every operation. Errors are OCaml `Invalid_argument` exceptions,
which leave `run` as thrown JS values.

## The `__proto__` phrase

npm's `_compress` keeps its dictionary in a plain object, `context_dictionary = {}`. When the phrase
`wc` is the string `__proto__`, `context_dictionary[context_wc] = context_dictSize++` reaches the
inherited `Object.prototype.__proto__` setter, which ignores a number. The phrase never becomes an
own property, but `context_dictSize` still increments, so that code number is spent. The port keeps
a `Hashtbl`, which would store the key, so it skips that one insertion and still advances the code
counter (`codec.ml:38`, `codec.ml:47`; `special` is `__proto__` in the two-byte encoding). This
reproduces npm; it does not special-case input. Only this name is affected: npm tests membership
with `hasOwnProperty`, so `constructor`, `toString` and the other `Object.prototype` names are
ordinary phrases, and the pending-literal object (`context_dictionaryToCreate`) only ever holds
single units. The corpus cases `keys-*` and `decode-keys-*` exercise it.

## Deviations from the npm source

- **Containers.** `context_dictionary` is a `(string, int) Hashtbl.t` and
  `context_dictionaryToCreate` a `(string, unit) Hashtbl.t` (`pending`), keyed by the two-byte
  phrases.
- **One bit writer.** npm repeats its bit loop for each case: an 8-bit literal, a 16-bit literal, a
  dictionary code, the end marker and the flush. The port has one `write` (the `n` low bits, least
  significant first) and one `emit` for `context_w`. `emit` sends either the pending literal (a 0
  or 1 marker in `numBits` bits, then 8 or 16 bits) or the dictionary code, and advances
  `enlargeIn` as npm's two decrements do.
- **Words, then characters.** The writer collects integer words of `bitsPerChar` bits, and the
  format maps them at the end: `raw` as is, `utf16` plus 32 with a trailing space, `base64` and
  `uri` through their alphabets (Base64 padded with `=` to a multiple of four), and `bytes` as high
  and low byte. npm maps each word through `getCharFromInt` as it is produced. The flush pads the
  last word with zero bits, or writes a whole zero word when the last word was just completed, as
  npm's flush loop does.
- **Decoder input.** Each format's units become words before decoding: `utf16` subtracts 32,
  `bytes` joins big-endian pairs, and `base64` and `uri` look each character up in the alphabet
  (`uri` reads a space as `+`). npm reads them lazily through `getNextValue`. A character outside
  the alphabet becomes 0 (npm's `getBaseValue` returns `undefined`, and `undefined & position` is
  0), and a read past the end gives 0 (npm's `NaN & position`).
- **Decoder dictionary.** A `(int, string) Hashtbl.t` from code 3 upward, with `next` as npm's
  `dictSize`. npm's first three slots hold the numbers 0, 1 and 2, which codes 0 to 2 never reach.
- **The undefined `w`.** A stream whose 2-bit header is 3 matches none of npm's `case`s, so `c`
  and `w` stay `undefined`. `dictionary[3]` is then `undefined`, the first output piece joins as an
  empty string, a code equal to `dictSize` throws a TypeError at `w.charAt(0)`, and otherwise the
  next phrase is `"undefined"` followed by the entry's first unit. The port represents that `w` as
  `""` (no real phrase is empty) and reproduces all three effects; the TypeError is the
  `Invalid_argument` that `unit_at "" 0` raises.
- **Results.** `None` (JS `null`) where npm returns `null`: an empty input, or a code beyond the
  dictionary. An empty result where npm returns `""`: the stream runs out before its end marker.
  The decode loop is a tail-recursive `loop`.
- **Odd byte count.** npm's `decompressFromUint8Array` throws a RangeError from
  `new Array(length / 2)`; the port raises `Invalid_argument` at the same point. The harness
  compares only that both threw.
- **`roundtrip` with null data.** The reference adapter throws, because it reads `.length` of the
  decoder's `null`. The port returns an empty array, as it does for null data in every operation.
  `roundtrip` is the benchmark operation, and CONTRACT.md excludes null data from it.

## Module state

None.

## Performance accommodations

- `compress` builds each unit's phrase from its two bytes (`symbol (unit_at s (2*i))`) instead of
  slicing it with `String.sub`, because Melange's `String.sub` converts the whole input to bytes
  before it slices (the comment at `codec.ml:40`).
