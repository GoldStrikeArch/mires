# marked 18.1.1: Dart port (dart2js)

A faithful port of the synchronous CommonMark + GFM + pedantic subset defined in
`libraries/marked/CONTRACT.md` in Dart 3 (`sdk: ^3.13.0`, compiled with Dart SDK 3.14.0 via
`languages/dart/build.mjs`: `dart compile js -O2 --no-source-maps`, comparator facade, common
Terser). It mirrors `marked@18.1.1`'s exact architecture (`Rules`, `Tokenizer`, `Lexer`,
`Renderer`, `TextRenderer`, `Parser`, `Marked`, and `helpers`) and reaches **80/80**
character-for-character parity in Node 26, Chrome 158, and Firefox 160 for both `dist/index.min.js`
and `dist/index.js` (`check.mjs --readable`).

| File | What it is |
| --- | --- |
| `src/options.dart` | `MarkedOptions` (`gfm`, `breaks`, `pedantic`) matching `defaults.ts` |
| `src/tokens.dart` | Sealed non-generic `Token` hierarchy (`SpaceToken`, `CodeToken`, `HeadingToken`, `HrToken`, `BlockquoteToken`, `ListToken`, `ListItemToken`, `CheckboxToken`, `HtmlToken`, `DefToken`, `TableToken`, `TableCellToken`, `ParagraphToken`, `TextToken`, `EscapeToken`, `LinkToken`, `ImageToken`, `StrongToken`, `EmToken`, `CodespanToken`, `BrToken`, `DelToken`), `LinkDef`, `InlineQueueEntry` |
| `src/rules.dart` | Immutable `MarkedRules` bundle (`final markedRules = MarkedRules._init();`) compiling all 127 block/inline rules (`normal`/`gfm`/`breaks`/`pedantic`) plus `other` rules and precomputed `0..3` indent regex caches via `_RuleEditor` (faithful port of `rules.ts`'s `edit()` including `other.caret` stripping) |
| `src/helpers.dart` | ECMA-262 whitespace trimming (`jsTrim`/`jsTrimStart`/`jsTrimEnd`), JS clamping slices (`jsSubstring`/`jsSlice`), `escapeHtmlEntities`, `decodeNumericCharacterReferences`, `cleanUrl`, `splitCells`, `rtrim`, `trimTrailingBlankLines`, `normalizeLabel`, `findClosingBracket`, `expandTabs` |
| `src/tokenizer.dart` | `Tokenizer` implementing all 13 block and 11 inline rules, `_outputLink`, `_indentCodeCompensation`, and `_isLabelEndInsideToken` |
| `src/lexer.dart` | `Lexer` and `LexerState`: `lex`, `blockTokens`, `inline`, `_linkInText`, `inlineTokens`, 3-pass delimiter masking (`reflinkSearch`, `anyPunctuationOutsideCode`, `blockSkip`), and FIFO `inlineQueue` draining |
| `src/renderer.dart` | `Renderer` emitting `marked@18.1.1`'s exact HTML for all block and inline token types |
| `src/text_renderer.dart` | `TextRenderer` flattening inline tokens for image `alt` attributes |
| `src/parser.dart` | `Parser` (`parse`, `parseInline`, `parseInlineText`) using exhaustive sealed-class pattern switches |
| `src/marked.dart` | `Marked` entry class (`parse`, `parseInline`) |
| `src/main.dart` | `dart:js_interop` boundary (port of `libraries/marked/reference.mjs`); `main()` publishes `run` on `globalThis.__matrixExports` |

## Architecture and interop decisions

- **Zero-copy JS string boundary.** On dart2js, Dart `String` is the host JS UTF-16 string, so
  `input.markdown` and the returned HTML `JSString` (`.toJS`) cross the boundary with zero copy or
  encoding conversion. Input objects are read via realm-safe `dart:js_interop` extension types
  (`InputJson`, `OptionsJson`) without `dartify()`.
- **Host `RegExp` grammar compiled verbatim.** Dart's `RegExp` on web compiles directly to JS
  `RegExp`, preserving exact ECMAScript `i`/`m`/`u` flag semantics, Unicode property escapes
  (`\p{P}`, `\p{S}`, `\p{Pi}`, `\p{Ps}`, `\p{L}`, `\p{N}`), negative lookbehind `(?<!`)()`, and
  named backreferences (`\k<a>`, `\k<b>` in `blockSkip`). All 127 compiled block/inline regexes
  were verified against `marked@18.1.1`'s `Lexer.rules` (exact pattern and flag match).
- **JS-vs-Dart standard library differences handled explicitly:**
  - **ECMA-262 whitespace (`U+0085`):** Dart's `String.trim()` trims `U+0085` (NEL), whereas JS's
    `String.prototype.trim()` does not. `jsTrim`/`jsTrimStart`/`jsTrimEnd` test the exact 25
    ECMA-262 whitespace/line-terminator code points.
  - **`cleanUrl` without `Uri.encodeFull`:** `Uri.encodeFull` maps lone surrogates to `%EF%BF%BD`
    instead of throwing `URIError` (returning `null`) and pulls `dart:core`'s `Uri` implementation
    into the bundle. A pure-Dart single-pass UTF-8 percent-encoder matches
    `encodeURI(href).replace(/%25/g, '%')` across all ASCII, BMP, astral, and lone-surrogate inputs.
  - **Clamped slicing (`jsSubstring`/`jsSlice`):** JS `substring`/`slice` clamp out-of-range indices
    (notably in `blockquote` list continuation where a trailing empty split can advance `raw.length`
    one past `src.length`, and in `list` line stepping `rawLine.length + 1`). Explicit clamping
    avoids Dart `RangeError`.
  - **Capture-group bounds:** Dart's `RegExpMatch[n]` throws `RangeError` when `n > groupCount`,
    unlike JS returning `undefined`. Call sites guard `nolink` (`groupCount == 1`),
    `inlinePedantic.link` (`groupCount == 2`), and `emStrongRDelimUnd` (`groupCount == 5`).
  - **Unicode code-point counting:** `expandTabs` advances columns per Unicode scalar value (handling
    UTF-16 surrogate pairs as width 1), and `emStrong`/`del` compute `lastCharLength` (`[...m[0]][0].length`)
    in O(1) directly from UTF-16 lead/trail surrogate code units.
  - **Unicode case folding:** `normalizeLabel` calls `jsTrim(label).toLowerCase().toUpperCase().toLowerCase()`,
    which uses the host JS string case mappings on dart2js.
- **No cross-call mutable state.** Every `run` call instantiates fresh `Marked`/`Lexer`/`Tokenizer`/`Parser`/`Renderer`
  objects with fresh `tokens`, `links`, and `inlineQueue`. Nothing is memoised across calls.

## Lint columns

- **Embedded JS:** `0`
- **Eval references:** `0`
- **Host JS:** `0`
- **Module state:** `1` — `final markedRules = MarkedRules._init();` (`src/rules.dart:6`). This
  single immutable object holds `marked@18.1.1`'s own module-level compiled `RegExp` grammar tables
  (including the 6 pre-populated `0..3` indent regex arrays matching `rules.ts`'s
  `cachedIndentRegex`). Bundling all rules into one `final` instance mirrors `ports/fflate/dart`
  and keeps dart2js to a single lazy-initializer check. No mutable state or input cache exists.

## dart2js 3.14 findings (no compiler bugs)

No compiler bug or refusal was hit. Profiling `dart compile js -O2` output on `bench-parse` and
`bench-gfm` guided four clean, idiomatic Dart optimizations that cut Node latency from ~1.27×–1.32×
JS down to ~1.01×–1.02× JS:

1. **First-character guards before anchored `^` `RegExp.firstMatch` calls.** Every failed
   `RegExp.firstMatch` in `dart2js` pays wrapper dispatch plus V8 regex entry. Checking the
   necessary starting UTF-16 code unit before calling each anchored tokenizer rule (e.g. `\` for
   `escape`, `<` for `tag`/`autolink`, `[`/`!` for `link`/`reflink`, `*`/`_` for `emStrong`, `` ` ``
   for `codespan`, `~` for `del`, and up-to-3-space lookahead for block rules) skips ~80% of doomed
   regex probes with zero semantic change.
2. **Lazy inline masking and hoisted mask callbacks.** `maskedSrc` in `Lexer._inlineTokensInner` is
   only consulted by `emStrong` and `del`. Computing it lazily on the first `*`, `_`, or `~` (and
   skipping `anyPunctuationOutsideCode` / `blockSkip` when their trigger characters are absent)
   avoids 3 global regex passes plus closure allocations on plain table cells, headings, and link
   texts. Hoisting `_maskAnyPunctuationOutsideCode` and `_maskBlockSkip` to top-level functions
   eliminated per-call closure allocation and RTI function-type checks.
3. **Range fast-path in `escapeHtmlEntities`.** All five escapable ASCII bytes (`"`, `&`, `'`, `<`,
   `>`) fall in `0x22..0x3E`; a single `if (c > 0x3E || c < 0x22) continue;` skips letters, spaces,
   newlines, and non-ASCII text in one check.
4. **Sealed non-generic `Token` hierarchy ordered hot-case-first.** Ordering `TextToken` first in
   `Parser.parseInline` and `ParagraphToken`/`TextToken` first in `Parser.parse` (with a 1-token
   fast path avoiding `StringBuffer`) keeps dart2js's `instanceof` dispatch short with zero `as`
   RTI casts.

## Size and speed

- **Size.** `dist/index.min.js` is **102,236 B** (**35,290 B** gzip-9, **30,283 B** brotli-11);
  `dist/index.js` is **115,956 B** (`dart2js` emits **115,634 B** before the facade and Terser).
  The Dart runtime floor is ~38.7 KB, leaving ~63.5 KB for the full `marked@18.1.1` grammar tables,
  lexer/tokenizer/parser/renderer, and `RegExp`/`StringBuffer`/`LinkedHashMap` support. For scale,
  the bundled JS reference (`reference.mjs` + `marked@18.1.1`) is **46,922 B** (**13,794 B** gzip-9).
- **Quick Node perf check** (`bench.mjs --environments node --samples 6`, triplet design, bench
  variants enabled):
  - `bench-parse` (52 KB mixed handbook): **~2.34 ms** vs JS **~2.32 ms** (**~1.01×** JS)
  - `bench-gfm` (46 KB GFM tracker, `breaks: true`): **~2.33 ms** vs JS **~2.28 ms** (**~1.02×** JS)

## Verification

- **Corpus parity:** `80/80` exact character-for-character parity in Node 26, Chrome 158, and
  Firefox 160 for both `dist/index.min.js` and `dist/index.js` (`check.mjs --readable`).
- **Rule table verification:** All 127 compiled block/inline regexes across `normal`, `gfm`,
  `breaks`, and `pedantic` match `marked@18.1.1`'s `Lexer.rules` patterns and flags.
- **Differential fuzzing:** A 2,736-case adversarial + randomized differential suite in Node against
  `libraries/marked/reference.mjs` passed `2,736/2,736` (0 failures), covering all 8
  `(gfm, breaks, pedantic)` option combinations and both `parse` and `parseInline`, including
  `U+0085` NEL vs ECMA-262 whitespace, astral characters, nested blockquote/list continuations,
  recursive `reflinkSearch` masking, raw-block restoration on rejected nested links, GFM `_backpedal`,
  and NCR/entity/URL-encoding edge cases.
- **Static analysis:** `dart analyze` reports 0 issues.
- **Reproducibility:** Two consecutive `--clean` builds are byte-identical
  (`reproducibility.identical === true`).
