# lz-string 1.5.0: Scala.js port

A hand-written port of `_compress`, `_decompress` and the ten format methods of
`node_modules/lz-string/libs/lz-string.js`, the source the reference wraps. LZ codes are written
least significant bit first into words whose first bit is the most significant, with npm's
`enlargeIn` / `numBits` growth, end marker and flush. Everything is in `src/Main.scala`.

## Boundary

`run(input)` is exported with `@JSExportTopLevel` and reads the corpus object through the native
facade trait `Input`: `data` is a `js.Array[Int]` of UTF-16 code units or bytes (or `null`), and
`repeat` a `js.UndefOr[Int]`. As in `reference.mjs`, the units become one string (`Char` per unit,
then `*` for `repeat`); a Scala.js `String` is a JS string, so isolated surrogates and NUL survive.
The result is a `js.Array[Int]`, or `null` where npm returns `null`. Null `data` gives an empty
array for every operation. Errors are Java exceptions (`IllegalArgumentException` from `require`
and `width`), thrown as JS values.

## The `__proto__` phrase

npm's `_compress` keeps its dictionary in a plain object, `context_dictionary = {}`. When the phrase
`wc` is the string `__proto__`, `context_dictionary[context_wc] = context_dictSize++` reaches the
inherited `Object.prototype.__proto__` setter, which ignores a number. The phrase never becomes an
own property, but `context_dictSize` still increments, so that code number is spent. The port keeps
a `mutable.HashMap`, which would store the key, so it skips that one insertion and still advances
the code counter (`Main.scala:67`). This reproduces npm; it does not special-case input. Only this
name is affected: npm tests membership with `hasOwnProperty`, so `constructor`, `toString` and the
other `Object.prototype` names are ordinary phrases, and the pending-literal object
(`context_dictionaryToCreate`) only ever holds single units. The corpus cases `keys-*` and
`decode-keys-*` exercise it.

## Deviations from the npm source

- **Containers.** `context_dictionary` is a `mutable.HashMap[String, Int]` and
  `context_dictionaryToCreate` a `mutable.HashSet[String]` (`fresh`).
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
- **Decoder dictionary.** A `mutable.ArrayBuffer[String]`. npm stores the numbers 0, 1 and 2 in its
  first three slots; codes 0 to 2 never reach the lookup, so the port leaves them `null`.
- **The undefined `w`.** A stream whose 2-bit header is 3 matches none of npm's `case`s, so `c`
  and `w` stay `undefined`. `dictionary[3]` is then `undefined`, the first output piece joins as an
  empty string, a code equal to `dictSize` throws a TypeError at `w.charAt(0)`, and otherwise the
  next phrase is `"undefined"` followed by the entry's first unit. The port represents that `w` as
  `null` and reproduces all three effects; the TypeError is `require(previous != null)`.
- **Results.** `decompress` returns `Option[String]`: `None` where npm returns `null` (an empty
  input, or a code beyond the dictionary), `Some("")` where npm returns `""` (the stream runs out
  before its end marker).
- **Odd byte count.** npm's `decompressFromUint8Array` throws a RangeError from
  `new Array(length / 2)`; the port's `require` throws an `IllegalArgumentException` at the same
  point. The harness compares only that both threw.
- **`roundtrip` with null data.** The reference adapter throws, because it reads `.length` of the
  decoder's `null`. The port returns an empty array, as it does for null data in every operation.
  `roundtrip` is the benchmark operation, and CONTRACT.md excludes null data from it.

## Module state

None.

## Performance accommodations

None.
