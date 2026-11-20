# marked 18.1.1: Scala.js port

A faithful port of the synchronous CommonMark and GFM compilation subset in
`libraries/marked/CONTRACT.md` in Scala 3.8.4 for Scala.js 1.22.0, built by
`languages/scalajs/build.mjs` (`fullLinkJS`, ES modules, common Terser). It implements
`marked@18.1.1`'s exact architecture — block and inline `Rules` (`normal`, `gfm`, `breaks`,
`pedantic`), `Helpers`, `Tokenizer`, `Lexer` (including 3-pass pre-scan delimiter masking and FIFO
`inlineQueue`), `Renderer`, `TextRenderer`, and `Parser` — reaching exact character-for-character
parity on all 80 corpus cases in Node, Chrome, and Firefox (both minified and readable artifacts).

| File | What it is |
| --- | --- |
| `src/JsInterop.scala` | Zero-overhead `@js.native` facades (`JsStr`, `LinkMap`, `JsArr`, `JsStringStatic`, `JsObjectStatic`), `inline` string/array extensions, surrogate-pair-aware `codePointLength`/`firstCodePointUtf16Length`, and lightweight ASCII `parseDec`/`parseHex` helpers |
| `src/Options.scala` | `MarkedOptions(gfm, breaks, pedantic)` matching the contract subset |
| `src/Tokens.scala` | Sealed `Token` hierarchy for all block/inline token types, `TableCell`, `LinkDef`, `InlineQueueEntry`, and per-call `LexerState` |
| `src/Rules.scala` | Exact `marked@18.1.1` `Editor` rule builder, all `other` helper regexes, immutable 4-slot indent-regex tables, and `BlockRules`/`InlineRules` for `normal`, `gfm`, `breaks`, and `pedantic` |
| `src/Helpers.scala` | `escapeHtmlEntities`, `decodeNumericCharacterReferences`, `cleanUrl`, `splitCells`, `rtrim`, `trimTrailingBlankLines`, `normalizeLabel`, `findClosingBracket`, `expandTabs` |
| `src/Tokenizer.scala` | All 13 block and 11 inline tokenizer methods plus `outputLink`, `indentCodeCompensation`, and `isLabelEndInsideToken` |
| `src/Lexer.scala` | `Lexer` (`lex`, `blockTokens`, `inline`, re-entrant `linkInText`, `inlineTokens`, 3-pass `inlineTokensInner` masking, and `Lexer` companion entry points) |
| `src/Renderer.scala` | `Renderer` (HTML emitters for all block and inline tokens) and `TextRenderer` (image `alt` flattening) |
| `src/Parser.scala` | `Parser` (`parse`, `parseInline`, `parseInlineText`) |
| `src/Main.scala` | `@js.native` facade traits (`Input`, `OptionsJson`), `Marked.parse`/`parseInline`, and `@JSExportTopLevel("run")` |

## Architecture & Interop Decisions

- **Host `js.RegExp` for all grammar rules (`Rules.scala`).** As observed in MiniSearch's Scala.js
  port, `java.util.regex` rejects Unicode `\p{...}` property escapes under the builder's default
  ES2015 link target (and differs from ECMAScript on lookbehind/named backreferences and UTF-16
  semantics). All block, inline, and helper patterns are compiled directly as `scala.scalajs.js.RegExp`
  values on `object Rules` using `marked@18.1.1`'s exact `edit()` builder and flags (`g`, `u`, `i`,
  `m`). Global `/g` regexes used with `.exec()` (`emStrongRDelimAst`, `emStrongRDelimUnd`,
  `delRDelim`, `blockSkip`, `reflinkSearch`) explicitly reset `lastIndex = 0` on entry, and recursive
  `linkInText` saves and restores `lastIndex` across nested calls.
- **ECMAScript String/Object semantics via `@js.native` facades (`JsInterop.scala`).** Java's
  `String.trim()` strips ASCII control characters `<= 0x20` rather than ECMA-262 whitespace
  (missing `\u00A0`, `\u2003`, `\u3000`, `\uFEFF`), and `java.lang.String.substring` throws on
  out-of-bounds offsets where JS `substring`/`slice` clamp silently (e.g. `Tokenizer.list` advancing
  past a final line without `\n`). Thin `@js.native` facades (`JsStr`, `JsArr`, `LinkMap`) compile
  directly to native JS methods with zero runtime wrapper checks or copies.
- **Null-prototype link table (`LinkMap` via `Object.create(null)`).** `Lexer.links` is created with
  `Object.create(null)` and accessed via `@JSBracketAccess` on `LinkMap` (plus `Object.hasOwn` in
  reflink masking), matching `marked`'s prototype-pollution safety on labels like `toString`,
  `constructor`, and `__proto__`, while avoiding `js.Dictionary.apply`'s throwing lookup and
  `scala.collection` wrapper overhead.
- **Avoiding `scala.collection` keeps the bundle compact.** Replacing `StringOps.nonEmpty`,
  `js.ArrayOps.jsSlice`, and varargs `splice` with `length > 0`, `arrSlice`, and `length = count`
  eliminated `scala.collection.StringOps` and heavy collection builders, keeping the linked
  Scala.js runtime/stdlib footprint at ~64.4 KB readable (matching `fflate`'s minimal floor) and the
  final minified artifact at **75.0 KB** (~1.60× the 46.9 KB JS reference).
- **Zero-allocation Unicode code-point counting.** `Tokenizer.emStrong` and `Tokenizer.del` count
  Unicode code points and first-code-point UTF-16 width with direct surrogate-pair checks
  (`codePointLength`, `firstCodePointUtf16Length`) rather than spreading strings into temporary
  arrays (`[...match[0]]`). `Helpers.expandTabs` likewise advances columns per Unicode scalar value
  with a fast zero-copy return when no `\t` is present.
- **No cross-call mutable state.** `object Rules` holds only read-only compiled `js.RegExp` values
  and immutable 4-element indent tables (pre-populated at module load for clamped indents `0..3`,
  matching `marked`'s `cachedIndentRegex`; `listItemRegex` is constructed per list just like
  `marked`). Every `run` call instantiates a fresh `Lexer`, `Tokenizer`, `Parser`, and `Renderer`.

## Lint Columns

- **Embedded JS:** `0`. **Eval references:** `0`. **Host JS:** `0`.
- **Module state:** `1` finding, a false positive on the abstract trait member `var raw: String`
  inside `sealed trait Token` (`src/Tokens.scala:9`), because `tools/lib/source-stats.mjs` matches
  any `var` indented `<= 2` spaces and does not distinguish `trait`/`class` bodies from `object`
  bodies. Zero `var`s or mutable collections exist on any `object`.

## Size Breakdown

Measured via `tools/sizes.mjs` (`fullLinkJS` + common Terser, `2,271` Scala code lines across 10
files):

- **`dist/index.min.js`:** `75,045` B raw / `22,805` B gzip-9 / `19,516` B brotli-11
  (JS reference for scale: `46,922` B raw / `13,794` B gzip-9 / `12,495` B brotli-11)
- **`dist/index.js` (readable linked output):** `195,583` B raw / `36,173` B gzip-9 / `30,416` B
  brotli-11
  - Port code (`marked.*`): `131,172` B (`67.1%`), led by `Tokenizer` (`43.1 KB`), `Rules$`
    (`24.9 KB`), `Lexer` (`17.4 KB`), `Helpers$` (`6.5 KB`), `Renderer` (`4.6 KB`), and `Parser`
    (`4.2 KB`).
  - Scala.js runtime/stdlib: `64,412` B (`32.9%`), matching the minimal Scala.js runtime floor
    observed in `fflate`.

## Verification & Quick Node Perf Observations

- **Reproducibility:** 2 consecutive `--clean` builds produced byte-identical artifacts
  (`reproducibility.identical === true`).
- **Corpus parity:** `80/80` exact character-for-character parity in Node 26, Chrome 158, and
  Firefox 160 for both `dist/index.min.js` and `dist/index.js`.
- **Differential fuzzing:** `1,960/1,960` adversarial + seeded random Markdown cases passed
  character-for-character against `libraries/marked/reference.mjs` in Node for both minified and
  readable bundles (covering all 8 `(gfm, breaks, pedantic)` combinations × `parse` and
  `parseInline`, `Object.prototype` link labels, Unicode case folding, astral/emoji tab expansion
  and flanking, nested blockquotes/lists/task items, GFM tables, all 7 HTML block types, inline
  raw/link state, NCR decoding, `_backpedal` URL trimming, and 40 `bench-variant` transforms).
- **Quick Node perf check** (`tools/bench.mjs --environments node --samples 6`, triplet design,
  load average ~3.5):
  - `bench-parse`: `2.704 ms` port vs `2.412 ms` JS — **`port/js = 1.12×`** (`paired = 1.09×`,
    `A/A = 1.122`)
  - `bench-gfm`: `2.410 ms` port vs `2.293 ms` JS — **`port/js = 1.05×`** (`paired = 1.07×`,
    `A/A = 1.011`)
  Delegating regex execution directly to host `js.RegExp` and avoiding `scala.collection` wrappers
  allows the Scala.js port to track upstream `marked@18.1.1` within ~5–12%.

## Scala.js 1.22 Findings

- No compiler bugs or linker issues encountered; Scala 3.8.4 union types (`T | Null`) and `inline`
  extension methods compiled cleanly without warnings and reproduced byte-identically across clean
  builds.
- `js.Dictionary.apply(key)` throws `java.util.NoSuchElementException` on missing keys instead of
  returning `undefined` (and drags in `WrappedDictionary`); a `@js.native` `@JSBracketAccess` trait
  returning `js.UndefOr[V]` provides idiomatic zero-overhead null-prototype dictionary access.
- Using `String.nonEmpty` silently pulls in `scala.collection.StringOps` because `nonEmpty` is an
  extension method on `StringOps` rather than a method on `java.lang.String`; using `length > 0`
  (or `isEmpty`) avoids that linkage.
