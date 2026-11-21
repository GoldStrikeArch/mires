# marked 18.1.1: Elm port

A port of the `marked@18.1.1` synchronous Markdown-to-HTML subset defined in
`libraries/marked/CONTRACT.md` to Elm 0.19.2: the block/inline `Lexer`, `Tokenizer`,
`Parser`, `Renderer`, `TextRenderer`, helpers, and CommonMark / GFM / pedantic rule precedence of
`node_modules/marked/lib/marked.esm.js` (and its embedded TypeScript sources). No kernel code, no
`Debug`, no packages beyond the default `elm/core 1.0.5` and `elm/json 1.1.3`, zero embedded JS,
zero `RegExp`, and a minimal 27-line I/O-only JS host (`js/index.js`, `mostlyJs: false`, 1.1%
comparator share). The port matches `marked@18.1.1` character for character on all 126 corpus
cases in Node, Chrome, and Firefox (both minified and readable bundles). The 2026-11-20 pre-merge
review found divergences in the hand-written scanners (regex `.` line terminators, the `{1,999}`
label bound counted in code points, and backtracking in four rules), and the 2026-11-21 re-review
found three more (optional-escape backtracking in `block.def`/`inline.link` titles, multi-candidate
`` ``+(?=\]) `` backtracking in `_inlineLabel`, and pedantic `<href>` astral `slice(1, -1)`
`cleanUrl` `Nothing` fallback, plus bare `href` alternative order before `(?=\))`); all are fixed,
and the review's probes, targeted suites, and differential fuzzers find no divergence.

| File | What it is |
| --- | --- |
| `src/Main.elm` | `Platform.worker` boundary: decodes `{ op, markdown, options }` with `Json.Decode`, runs `Lexer` + `Renderer`, returns `{ ok: html }` or `{ error: message }` through `response` |
| `src/Unicode.elm` | Immutable Unicode 17.0 range tables generated from `libraries/marked/unicode-classes.json` (`PUNCT`, `OPEN_QUOTE`, `ALNUM_BMP`, ECMA-262 `\s`), ASCII bitmasks, binary search, and the UTF-16-aligned `Src` buffer |
| `src/Tokens.elm` | `Options`, `Mode`, `Align`, `LinkDef`, `InlineSlot`, `TableCell`, `ListItem`, `BlockToken`, and `InlineToken` |
| `src/Helpers.elm` | `escapeHtmlEntities`, `decodeNumericCharacterReferences`, `cleanUrl`, `splitCells`, `rtrimChar`, `isBlankLine`, `trimTrailingBlankLines`, `normalizeLabel`, `collapseSpaces`, `findClosingParenSpan`, `expandTabs`, `unescapeAnyPunctuation`, `unescapeLinkBrackets`, `normalizeNewlines` |
| `src/BlockRules.elm` | Deterministic pure-Elm scanners for every `marked@18.1.1` block rule across CommonMark, GFM, and pedantic modes (space, indented/fenced code, ATX & setext headings, hr, blockquote & lazy continuation, list items/tabs/indentation, 7-type HTML blocks, link defs, GFM tables, paragraphs) |
| `src/InlineRules.elm` | Deterministic pure-Elm scanners for every `marked@18.1.1` inline rule (escape, tag, link/reflink/nolink, `isLabelEndInsideToken`, emStrong `*`/`_` flanking with punctuation/quote/alnum rules, codespan, br, del, autolink, GFM URL/email autolink with `_backpedal`, inline text) plus 3-pass `buildMaskedCodes` pre-scan delimiter masking |
| `src/Lexer.elm` | Pure functional `Lexer` state machine: `lex`, `lexInline`, `blockTokens`, `inlineTokens`, FIFO `inlineSlots` queue draining, and all block/inline tokenizers matching `Lexer.ts` and `Tokenizer.ts` |
| `src/Renderer.elm` | `renderBlocks`, `renderInline`, and `renderTextInline` (`Parser.ts`, `Renderer.ts`, `TextRenderer.ts`) accumulating fragments into a reversed `List String` joined once |
| `js/index.js` | Minimal I/O-only JS host: sends `input` through the `request` port, receives the synchronous `response`, throws `TypeError` on `error`, and returns `response.ok` directly |

## Decisions

- **UTF-16-aligned `Src` buffer (`{ str : String, codes : Array Int, len : Int }`).** Input strings
  are converted once per `blockTokens` / `inlineTokens` entry via `String.foldl` (avoiding
  `elm/core 1.0.5`'s `String.foldr`/`String.toList` lone-surrogate bugs) into an `Array Int`
  aligned 1-to-1 with JS UTF-16 indices: BMP scalars occupy 1 slot, and astral scalars (`> 0xFFFF`)
  occupy 2 slots (the full code point at index `i` and its UTF-16 low surrogate `0xDC00..0xDFFF` at
  index `i + 1`). This gives O(1) `String.slice start end src.str`, exact UTF-16 `linkLen`
  truncation in `tokLink` (e.g. `[x](  é))`, `[x](    日本語))`), exact UTF-16 `prevChar =
  token.raw.slice(-1)` in `emStrong`/`del` (astral characters naturally expose their low surrogate,
  matching neither `alnum` nor `punct`), and aligned `maskedCodes` offsets without reslicing
  suffix strings on every token.
- **Native deterministic scanners + `unicode-classes.json` tables.** Elm's default packages do not
  include a regex engine (`elm/regex` also lacks the `u` flag), so all CommonMark, GFM, and
  pedantic grammars are implemented natively as hand-written cursor scanners. A scanner moves
  forward and must reproduce the regex's backtracking by reasoning about where the regex can
  succeed (the optional `|` of `lheadingGfm`'s table lookahead, `pedanticHrefTitle`'s `[^'"]*`, the
  pedantic `def`'s `<?`, `reflinkSearch`'s `nolink(?!\()`, optional delimiter escapes
  `\\"?`/`\\'?`/`\\\)?` in `block.def`/`inline.link` titles, LIFO multi-candidate `` ``+(?=\]) ``
  in `_inlineLabel`, and `inline.link` bare `href` precedence over `(?=\))`, each commented at the
  scanner), so equivalence rests on the corpus and on differential testing, not on construction. Unicode classes (`\p{P}\p{S}`,
  `[\p{Pi}\p{Ps}"']`, BMP `[\p{L}\p{N}]`, and ECMA-262 `\s`) use ASCII bitmasks plus binary search
  over sorted `Array Int` tables generated from `libraries/marked/unicode-classes.json`. HTML tag
  tables (`HTML_BLOCK_TAGS`, `HTML_TYPE1_TAGS`, `PEDANTIC_INLINE_TAGS`) are written as `case`
  expressions over lowercase strings, compiling to JS `switch` statements.
- **Regex `.`, `m`-flag lines and UTF-16 counts.** No marked rule has the `s` flag, so a regex `.`
  matches any code unit except the line terminators `\n`, `\r`, U+2028 and U+2029. Scanners
  that stand for `.` (heading, lheading, `def`'s `<.*?>`, the GFM table header and rows, the link
  `<...>` escape, the pedantic link and `pedanticHrefTitle`) stop at all four via
  `BlockRules.dotRunEnd` / `Unicode.isLineTerminator`; scanners for `[^\n]` use `lineEnd`. The
  three `m`-flag rules (`codeRemoveIndent`, `blockquoteSetextReplace2`, `spaceLine`) go through
  `BlockRules.mapJsLines`, which splits at all four. `parse` turns `\r` into `\n`, `parseInline`
  keeps it. Counts are UTF-16 code units because `Src` is UTF-16 aligned; the one bound that
  counts characters, `_boundedBlockLabel`'s `{1,999}`, adds `charWidth` per item, so an astral
  character is two.
- **Upstream behaviours kept.** Preserves upstream nuances: `breaks` requires
  `gfm && !pedantic` while task lists check `options.gfm` directly; `indentCodeCompensation`
  applies only to backtick fences; paragraph HTML interrupts are case-sensitive while `block.html`
  and list HTML breaks are case-insensitive; `Dict String LinkDef` handles prototype keys
  (`__proto__`, `constructor`, `toString`, `hasOwnProperty`) cleanly; `normalizeLabel` uses Elm's
  `String.trim`, `String.toLower`, and `String.toUpper` (compiled directly to the host's native
  Unicode case-folding methods); `cleanUrl` returns `Maybe String`, percent-encoding UTF-8 bytes of
  non-`encodeURI` scalars (keeping `%` verbatim) and returning `Nothing` on lone UTF-16 surrogates
  (including `Tokenizer.ts:809`'s pedantic `<href>` `slice(1, -1)` splitting a trailing astral
  character, so `Renderer` emits plain/escaped text); and `tokList` searches `inlineSlots` backwards
  when stripping task checkbox prefixes.
- **Stack-safe tail recursion and O(1) hot-path design.** Elm 0.19.2 applies tail-call optimization
  only to direct self-recursion (not mutual recursion). Both `blockTokensLoop` and
  `inlineTokensLoop` delegate single-token rule dispatch to non-recursive `stepBlockToken` /
  `stepInlineToken` helpers and recurse directly on themselves, compiling to flat JS `while` loops.
  All character, line, list, table, delimiter, and renderer loops are likewise directly
  self-tail-recursive. Tokens are accumulated in reversed lists for O(1) tail inspection/merging,
  HTML output is threaded through a single reversed `List String` joined once with `String.concat`,
  `maskedCodes` is computed lazily within each `inlineTokens` call only when `*`, `_`, or `~` is
  encountered, and hot integer comparisons compare differences against `0` (`a - b < 0`,
  `a - b == 0`) to avoid Elm's generic `_Utils_cmp` / `_Utils_eq` calls.
- **No `Infinite loop` error (declared deviation).** marked throws `Infinite loop on byte: N` when
  no block or inline rule consumes input. `blockTokensLoop` and `inlineTokensLoop` stop silently
  instead (`BlockDone` / `InlineDone`, or a step that does not advance): Elm has no exceptions,
  and surfacing the error would mean threading a `Result` through every recursive lexer entry. No
  string input reaches either condition, in marked or here: the block `text` rule (`^[^\n]+`)
  and `space` take any non-empty source, and `inlineText` (`^(`+|[^`])...`) takes at least one
  code unit.
- **Zero cross-call state.** The `Platform.worker` model is `()`, and `js/index.js` has only the
  standard two host module-state lines (`let app` and `let response`, identical to `pilot`,
  `minisearch`, and `fflate`). Every `run(input)` builds a fresh `Lexer` from its own input.

## Size and performance

- After the 2026-11-20 review fixes `dist/index.min.js` is `101,368 B` (`32,958 B` gzip-9,
  `26,748 B` brotli-11); the other figures in this section are from the build before them.
- `dist/index.min.js`: `101,212 B` (`32,868 B` gzip-9, `26,675 B` brotli-11; `9,249` Elm code
  lines, `0` embedded JS, `0` eval refs, `27` host-JS lines / `1,125 B`, `mostlyJs: false` at
  `1.11%` comparator share / `0.26%` rendered share, `2` host module-state lines). By `meta.json`,
  an empty `Platform.worker` is `6,484 B` minified; the remaining ~95 KB is the port, its Unicode
  range tables, and `elm/core` + `elm/json` (`Array`, `Dict`, `Bitwise`, `Json.Decode`/`Encode`).
- Local Node benchmark (`bench.mjs --environments node --samples 6`, rotating bench variant on,
  load average ~3.3–3.6):
  - `bench-parse` (51.8 KB mixed handbook): `11.30 ms` vs JS `2.23 ms` (`port/js` `5.06×`, `A/A` `0.978`)
  - `bench-gfm` (45.7 KB GFM tracker, `breaks: true`): `9.09 ms` vs JS `2.30 ms` (`port/js` `3.95×`, `A/A` `0.983`)

## Verification beyond the corpus

After the 2026-11-21 re-review fixes: `126/126` corpus parity in Node, Chrome, and Firefox (both
`index.min.js` and `index.js`), byte-for-byte clean rebuild reproducibility, plus 0 divergences on
`re-marked` probes (`probe-audit.mjs`, `probe-audit-2.mjs`, `targeted.mjs` / `targeted-cases.mjs`,
`earlier.mjs`), `fuzz-re.mjs`, and 30,000 targeted differential fuzz inputs.

After the 2026-11-20 review fixes: `111/111` corpus parity in Node, Chrome, and Firefox (both
artifacts); B1's differential fuzzers (194,000 inputs), B2's (1.09 million inputs: random fuzz plus
generators for HTML blocks, links and labels, pedantic, block structure, emphasis, GFM URLs,
fences, Unicode whitespace and stress sizes) and a targeted fuzzer for the fixed classes (400,000
inputs) all find no divergence, and 100 KB inputs of up to 50,000 tokens match marked.

Before the review, with the 80-case corpus, the port's own verification was exact `80/80` corpus
parity in Node, Chrome, and Firefox (for both `index.min.js` and `index.js`) and
`reproducibility.identical === true` across clean rebuilds, plus:
- `1,024` adversarial cases (`64` edge-case documents × all `8` `(gfm, breaks, pedantic)` combos ×
  `parse`/`parseInline`): `1,024/1,024` exact match.
- `2,700` randomized & grammar-targeted differential fuzz cases (HTML/pedantic HTML, links/defs/
  reflinks/extra-`)` truncation, emphasis/del/Unicode/astral chars, lists/blockquotes/tables/
  fences, GFM autolinks/`_backpedal`/breaks/NCR) across all `8` option combos & both ops:
  `2,700/2,700` exact match.
- `20` bench-variant checks (`k = 1..10` on both `bench-parse` and `bench-gfm`), unknown-`op`
  `TypeError` check, and 4,000-line / 4,000-token stack-safety stress tests: all exact match.
