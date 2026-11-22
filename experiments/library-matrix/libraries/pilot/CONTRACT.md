# Library contract: `pilot`

The pilot is the harness's proof library. It stands in for an npm package so that every toolchain,
the checker, the timing protocol and the report can be exercised end to end before real library
ports exist. It is small on purpose, but it touches the things real ports trip over: UTF-8 and
astral code points across the JS boundary, a hash map, a stable sort with a tie-break, 32-bit
unsigned integer arithmetic, and fixed-decimal float formatting.

| Field | Value |
| --- | --- |
| Package | `pilot-wordfreq` 1.0.0, a stand-in: `package/index.mjs` (not published) |
| Reference adapter | `reference.mjs`: `run(input) = analyse(input.text, { top: input.top })` |
| API subset | `analyse(text, { top })`, the package's only analysis entry |
| Input | `{ text: string, top: integer >= 0 }`; `text` is well-formed UTF-16 (no lone surrogates) |
| Output form | `string`, synchronous |
| Normalisation | none |
| Parity environments | node, chrome, firefox, safari |
| Corpus | `corpus.json`, 11 cases |
| Bench case | `bench` (3,577 UTF-16 units, 782 tokens) |
| Bench variant | `bench-variant.mjs`: the k-th variant rotates the words by k and replaces a k-chosen word with `v<k in base 36>`, so every prefix and suffix changes with k (779 to 783 tokens, inside the parity domain) |
| Caching | none: the package keeps no state between calls, so ports may not either |
| Files | none read at run time |

## Behaviour

1. **Tokens.** Walk `text` by code point. A *word code point* is an ASCII letter, an ASCII digit,
   or any code point at or above U+0080 (so U+00A0, U+3000 and the em dash are word code points).
   A token is a maximal run of word code points.
2. **Case folding.** ASCII only: `A`–`Z` become `a`–`z`. Every other code point is kept as it is
   (`CAFÉ` → `cafÉ`, `ПРИВЕТ` stays). No Unicode case mapping anywhere.
3. **Counts.** Count each distinct token. Its *length* is its number of code points.
4. **Ranking.** Sort the distinct tokens by count descending, then length descending; tokens equal
   on both keep the order in which they first appeared (a stable sort over first-occurrence order).
5. **Hash.** FNV-1a 32 over the UTF-8 bytes: offset `0x811c9dc5`, prime `0x01000193`, every step
   reduced modulo 2^32 (JavaScript `Math.imul(h ^ byte, prime) >>> 0`). Printed as 8 lowercase hex
   digits of the unsigned value.
6. **Floats.** `mean` = total code points / tokens (0 when there are no tokens), and each row's
   share = count / tokens, each printed like JavaScript `Number.prototype.toFixed(3)`.
7. **Result.** Lines joined by `\n`, no trailing newline:

   ```text
   tokens <tokens>
   distinct <distinct tokens>
   mean <mean>
   fnv <hash of the whole text>
   <word> <count> <hash of the word> <share>      one line per ranked token, the first `top` of them
   ```

## Parity domain and its one restriction

Parity is exact string equality on every corpus case, with no normalisation.

`toFixed` rounds an exact tie half-up, while several languages' own fixed-decimal formatters
(Rust `{:.3}`, Go `strconv.FormatFloat`, C-library `printf`) round ties to even. A share or mean is a
ratio of integers `a / tokens`; it can be an exact decimal tie at three digits only when `tokens` is
a multiple of 16 (a tie is `(2n+1)/2000`, whose reduced denominator always holds 2^4). Away from a
tie every correctly rounded formatter, and `round(x * 1000)` on these ratios, agrees with
`toFixed(3)`. **The parity domain therefore excludes token counts that are multiples of 16**; every
corpus case respects it (17, 7, 15, 13, 20, 5, 3, 6 and 782 tokens). A library whose real behaviour
reaches ties must put tie cases in its corpus, and its ports must then reproduce half-up rounding.

## Corpus

| Case | What it exercises |
| --- | --- |
| `empty` | no tokens: mean defined as 0, hash of zero bytes |
| `ascii` | case folding, punctuation as separators, ties broken by first occurrence |
| `ties` | equal counts: the explicit length key, then first-occurrence order |
| `unicode-bmp` | Latin-1, Cyrillic, Greek, CJK; non-ASCII letters not folded; the em dash as a token |
| `astral` | emoji, mathematical alphanumerics, CJK extension B, a flag pair, a ZWJ sequence |
| `punctuation-digits` | digits, `_`, `-`, `'`, brackets and `@` as separators |
| `whitespace` | ASCII whitespace separates; U+00A0 and U+3000 join a token |
| `top-zero` | header lines only |
| `top-large` | `top` above the number of distinct tokens |
| `wide-top` | `top` past 32-bit range (`4294967296`), valid non-negative integer |
| `bench` | 160 phrases drawn from the others by a seeded LCG; also the timing input |

## Ports

`ports/pilot/<language>/`, one per language, each idiomatic for its language. Each port's
`port.json` states its boundary (how `input` becomes the language's own values and how the result
goes back). The difficulties each language met are listed in the harness README ("Per-language
notes").
