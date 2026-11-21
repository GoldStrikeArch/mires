# lz-string 1.5.0: Elm port

A hand-written port of `_compress`, `_decompress` and the ten format methods of
`node_modules/lz-string/libs/lz-string.js`, the source the reference wraps. LZ codes are written
least significant bit first into words whose first bit is the most significant, with npm's
`enlargeIn` / `numBits` growth, end marker and flush.

| File | What it is |
| --- | --- |
| `src/Codec.elm` | `compress` and `decompress` over lists of UTF-16 code units |
| `src/Main.elm` | The port worker: decodes the input, dispatches the operation, encodes the result |
| `js/index.js` | The host: synchronous transport between `run` and the worker's ports |

## Boundary

`js/index.js` creates the Elm worker on the first call and sends each input on the `request` port;
the worker answers synchronously on `response` with `{ok}` or `{error}`, and the host throws a
`TypeError` for `{error}`. The host does no other work. `Main.elm` decodes `data` with
`Json.Decode.list Json.Decode.int` (UTF-16 code units or bytes, integers) and `repeat` with
`Json.Decode.int` (default 1). Null `data` fails that decoder and gives an empty array for every
operation. Text never becomes an Elm `String`: phrases are lists of code units, so isolated
surrogates and NUL survive. The result is a list of numbers, or `null` where npm returns `null`.
Elm has no exceptions, so the codec returns `Result String (Maybe (List Int))`: `Err` for npm's
throws, `Ok Nothing` for npm's `null`.

## The `__proto__` phrase

npm's `_compress` keeps its dictionary in a plain object, `context_dictionary = {}`. When the phrase
`wc` is the string `__proto__`, `context_dictionary[context_wc] = context_dictSize++` reaches the
inherited `Object.prototype.__proto__` setter, which ignores a number. The phrase never becomes an
own property, but `context_dictSize` still increments, so that code number is spent. The port keeps
an Elm `Dict`, which would store the key, so it skips that one insertion and still advances the
code counter (`Codec.elm:72-73` builds `special`, `Codec.elm:90` skips it). This reproduces npm; it
does not special-case input. Only this name is affected: npm tests membership with
`hasOwnProperty`, so `constructor`, `toString` and the other `Object.prototype` names are ordinary
phrases, and the pending-literal object (`context_dictionaryToCreate`) only ever holds single
units. The corpus cases `keys-*` and `decode-keys-*` exercise it.

## Deviations from the npm source

- **Containers.** `context_dictionary` is a `Dict (List Int) Int` keyed by the phrase's code units
  in reverse, so extending `w` by `c` is `c :: w`; `special` is reversed to match.
  `context_dictionaryToCreate` is a `Set Int` of units, which is the same set because it only ever
  holds single-unit phrases. `emit` reads the code with `Maybe.withDefault 0`; `w` is always in the
  dictionary, so the default is unreachable.
- **One bit writer.** npm repeats its bit loop for each case: an 8-bit literal, a 16-bit literal, a
  dictionary code, the end marker and the flush. The port has one `write` (the `n` low bits, least
  significant first) and one `emit` for `context_w`. `emit` sends either the pending literal (a 0
  or 1 marker in `numBits` bits, then 8 or 16 bits) or the dictionary code, and advances
  `enlargeIn` as npm's two decrements do. The encoder state is a record threaded through
  `List.foldl`, and finished words are consed and reversed at the end.
- **Words, then characters.** The writer collects integer words of `bitsPerChar` bits, and the
  format maps them at the end: `raw` as is, `utf16` plus 32 with a trailing space, `base64` and
  `uri` through their alphabets (Base64 padded with `=` to a multiple of four), and `bytes` as high
  and low byte. npm maps each word through `getCharFromInt` as it is produced. The flush pads the
  last word with zero bits, or writes a whole zero word when the last word was just completed, as
  npm's flush loop does.
- **Decoder input.** Each format's units become words before decoding (`unpack`): `utf16`
  subtracts 32, `bytes` joins big-endian pairs, and `base64` and `uri` look each character up in
  the alphabet (`uri` reads a space as `+`). npm reads them lazily through `getNextValue`. A
  character outside the alphabet becomes 0 (npm's `getBaseValue` returns `undefined`, and
  `undefined & position` is 0), and a read past the end gives 0 (npm's `NaN & position`).
- **Decoder dictionary.** An `Array (Maybe Phrase)`, where a `Phrase` keeps its first unit beside
  its units. npm stores the numbers 0, 1 and 2 in its first three slots; codes 0 to 2 never reach
  the lookup, so the port stores `Nothing` there. `decode` is tail-recursive.
- **The undefined `w`.** A stream whose 2-bit header is 3 matches none of npm's `case`s, so `c`
  and `w` stay `undefined`. `dictionary[3]` is then `undefined`, the first output piece joins as an
  empty string, a code equal to `dictSize` throws a TypeError at `w.charAt(0)`, and otherwise the
  next phrase is `"undefined"` followed by the entry's first unit. The port represents that `w` as
  `Nothing` (`previous : Maybe Phrase`) and reproduces all three effects; the TypeError is
  `Err "undefined phrase"`, which the host throws.
- **Results.** `Ok Nothing` where npm returns `null`: an empty input, or a code beyond the
  dictionary. `Ok (Just [])` where npm returns `""`: the stream runs out before its end marker.
- **Odd byte count.** npm's `decompressFromUint8Array` throws a RangeError from
  `new Array(length / 2)`; `unpackBytes` returns `Err`, and the host throws. The harness compares
  only that both threw.
- **`roundtrip` with null data.** The reference adapter throws, because it reads `.length` of the
  decoder's `null`. The port returns an empty array, as it does for null data in every operation.
  `roundtrip` is the benchmark operation, and the corpus never passes it null.

## Module state

The host keeps the worker instance (`app`) and the last reply (`reply`), reset before each call.
The worker's model is `()`, so nothing carries over between calls.

## Performance accommodations

None.
