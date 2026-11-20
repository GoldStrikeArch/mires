# marked 18.1.1: Go port (GopherJS)

A faithful port of the synchronous CommonMark + GFM Markdown-to-HTML compiler subset defined in
`libraries/marked/CONTRACT.md` to Go 1.21, compiled by GopherJS 1.21.0
(`gopherjs build -m --source_map=false`, the harness's production setting). It mirrors
`node_modules/marked/lib/marked.esm.js` (and its embedded `src/*.ts` sources in `marked.esm.js.map`)
module by module across all three grammar modes (`normal`, `gfm`/`breaks`, `pedantic`) and both
`parse` and `parseInline`, using only Go and `github.com/gopherjs/gopherjs/js` (no embedded JS,
no eval refs, no host JS, no standard-library packages beyond `runtime`, no goroutines, and no
64-bit integers).

| File | What it is |
| --- | --- |
| `src/options.go` | `Options`, `TokenKind` / `AlignKind` enums, `LinkDef`, `TableCell`, and concrete `Token` AST struct |
| `src/rules.go` | Single immutable package-level `var (` block holding compiled `other`, `block.{normal,gfm,pedantic}`, and `inline.{normal,gfm,breaks,pedantic}` host `RegExp` tables (all 114 marked rules) and read-only JS string constants initialised once in `init()` |
| `src/helpers.go` | `escapeHtmlEntities`, `decodeNumericCharacterReferences`, `cleanUrl`, `splitCells`, `rtrim`, `trimTrailingBlankLines`, `normalizeLabel`, `findClosingBracket`, `expandTabs`, `expandLeadingTabs`, UTF-16 code-point helpers, and `intToString` (no `strconv`) |
| `src/tokenizer.go` | `Tokenizer` block & inline methods (`space`, `code`, `fences`, `heading`, `hr`, `blockquote`, `list`, `html`, `def`, `table`, `lheading`, `paragraph`, `text`, `escape`, `tag`, `link`, `reflink`, `emStrong`, `codespan`, `br`, `del`, `autolink`, `url`, `inlineText`), `outputLink`, `indentCodeCompensation`, `isLabelEndInsideToken` |
| `src/lexer.go` | `Lexer` (`lex`, `lexInline`, `blockTokens`, `inlineTokens`, FIFO `inlineQueue`, `linkInText`, re-entrant pre-scan delimiter masking for reflinks, escapes/code spans, and `blockSkip`) |
| `src/renderer.go` | `Renderer` (all block and inline HTML emitters) and `TextRenderer` (image `alt` flattening) |
| `src/parser.go` | `Parser` (`parse`, `parseInline`, `parseInlineText` via concrete `switch` dispatch) |
| `src/boundary.go` | Corpus input decoding (`readOptions`, `op` dispatch, error on unknown `op`) |
| `src/main.go` | Publishes `run` on `js.Module.exports` |

## Boundary and interop decisions

- **Host `RegExp` grammar tables (exact `marked@18.1.1` rules).** Go's `regexp` (RE2) does not
  support lookahead, lookbehind (`(?<!`)`), or numbered/named backreferences (`\1`, `\2`, `\k<a>`,
  `\k<b>`) used in `marked`'s `rules.ts`, and would pull substantial stdlib weight. As explicitly
  permitted by `CONTRACT.md`, `src/rules.go` compiles `marked@18.1.1`'s exact grammar rules via
  `js.Global.Get("RegExp").New(pattern, flags)` once in `init()`. An automated check verified
  0 mismatches across all 114 rules (52 unique `RegExp`s) against `marked`'s `Lexer.rules`.
- **JS primitive strings (`type jsStr = *js.Object`) across the pipeline.** In GopherJS, Go `string`
  values store UTF-8 bytes inside a JS string; passing a Go `string` to any JS `RegExp` call triggers
  `$externalize(..., $String)`, which runs `$isASCII` across the entire string and, when any
  non-ASCII byte is present (as in `bench-parse`), decodes every rune into a new JS string on every
  `exec`/`test` call — turning the block/inline lex loops into $O(N^2)$ UTF-8/UTF-16 re-encoding.
  Keeping Markdown/HTML strings as `jsStr` (`*js.Object` holding JS primitive strings) eliminates
  that overhead completely and guarantees exact UTF-16 code-unit semantics for:
  - `Tokenizer#link` extra-`)` truncation (`linkLen = start + cap[1].length + lastParenIndex`,
    handling `[x](  é))` and `[x](    日本語))` identically to `marked`),
  - `maskedSrc` end-relative alignment with `src`,
  - astral `prevChar` (`token.raw.slice(-1)` yielding the trailing low surrogate),
  - full ECMA-262 whitespace in `.trim()` / `.trimStart()` / `.trimEnd()`, and
  - Unicode case folding in `normalizeLabel` (`trim().toLowerCase().toUpperCase().toLowerCase()`).
- **Prototype-safe link reference map.** `Lexer.links` is a Go `map[string]*LinkDef` keyed by the
  normalized label string (`.String()`), naturally avoiding JS prototype collisions (`__proto__`,
  `constructor`, `toString`, `hasOwnProperty`, `valueOf`).
- **No JS callback closures.** Every `marked` `.replace(re, fn)` site (`escapeHtmlEntities`,
  `decodeNumericCharacterReferences`, `splitCells`, `leadingSpaceTab`, `reflinkSearch`,
  `anyPunctuationOutsideCode`, `blockSkip`) is written as a Go loop calling `exec` (or `indexOf`)
  with explicit `lastIndex` management — saving and restoring `lastIndex` around recursive calls
  in `maskReflinks` and `linkInText` to match ECMA-262 `RegExp.prototype[@@replace]` and `matchAll`.
- **`cleanUrl` without `defer`/`recover`.** `encodeURI` throws `URIError` exclusively on unpaired
  UTF-16 surrogates. Pre-checking with `/[\uD800-\uDBFF](?![\uDC00-\uDFFF])|(?<![\uD800-\uDBFF])[\uDC00-\uDFFF]/`
  returns `nil` before invoking `encodeURI`, avoiding `defer`/`recover` overhead in GopherJS.
- **Module state lint (`moduleState: 1`).** `src/rules.go` line 125 (`var (`) is the sole
  package-level `var` declaration in the port. It holds only read-only compiled `RegExp` rule tables,
  host constructor references, and pre-allocated JS string constants initialised once in `init()` and
  never mutated afterwards, matching `marked@18.1.1`'s module-load rules (`CONTRACT.md`, "Caching").
  Shared `/g` regexes have `lastIndex` explicitly reset (and saved/restored across recursion) before
  every scan. Zero cross-call mutable state or input memoisation exists.

## GopherJS 1.21 findings

Applying the MiniSearch and `fflate` GopherJS lessons kept the emitted package (`matrix/marked/src`)
at **0 `$restore` state machines**, **0 `$indexPtr` calls**, **0 `$growSlice` calls**,
**0 `$clone` calls**, and **0 emulated 64-bit integer operations**:

1. **Zero closures, func variables, or interfaces (`0` `$restore`).** All AST tokens share one
   concrete `*Token` struct dispatched via `switch tok.kind`; `Parser` has concrete `parseInline`
   and `parseInlineText` loops rather than a `Renderer`/`TextRenderer` interface; helper builders
   in `rules.go` (`mkListItemRe`, `createParagraphRe`) are top-level functions rather than local
   closures.
2. **Zero struct cloning (`0` `$clone`).** `Options` is passed by pointer (`*Options`),
   `TextRenderer` methods use pointer receivers (`*TextRenderer`), and `Lexer.state` is initialised
   field-by-field.
3. **Zero slice-element pointers (`0` `$indexPtr`).** `tokens`, `items`, `inlineQueue`, and table
   cells are all slices of pointers (`[]*Token`, `[]*inlineQueueEntry`, `[]*TableCell`).
4. **No stdlib bloat.** Concrete decimal/hex parsers in `helpers.go` (`parseDecJS`, `parseHexJS`,
   `intToString`) avoid `strconv` and `fmt`, keeping the linked packages to just `runtime`,
   `github.com/gopherjs/gopherjs/js`, and `matrix/marked/src`.

`gofmt` and `go vet` are clean.

## Verification

- **Corpus parity (`tools/check.mjs --readable`):** **80/80** exact character-for-character parity
  in Node 26.8.1, Chrome 158, and Firefox 160 for both `dist/index.min.js` and `dist/index.js`.
- **Reproducibility:** two consecutive `tools/build.mjs --clean` builds produced byte-identical
  artifacts (`reproducibility.identical === true`).
- **Differential & adversarial testing:** **2,809/2,809** cases passed (0 failures) against
  `libraries/marked/reference.mjs` across all 8 `(gfm, breaks, pedantic)` combinations for both
  `parse` and `parseInline` (covering pedantic task lists, `<foo>bar>` defs, prototype keys,
  `[x](  é))` / `[x](    日本語))` UTF-16 truncation, nested lists/blockquotes/tables/emphasis/HTML,
  astral Unicode flanking & tab expansion, GFM autolinks/`_backpedal`, NCRs, unknown `op` throwing,
  and 1,200 randomized fuzz documents).

## Size

From `tools/sizes.mjs` (`build/library-matrix/dev-marked-gopherjs/receipts/sizes.json`):

- `dist/index.min.js`: **129,387 B** raw, **34,201 B** gzip-9, **29,836 B** brotli-11
  (against the GopherJS runtime floor of ~41.7 KB raw / ~13.7 KB gzip-9, and JS reference
  46,922 B raw / 13,794 B gzip-9).
- `dist/index.js`: **158,674 B** raw, **38,283 B** gzip-9, **32,856 B** brotli-11.
- Source lints: `3,240` Go code lines across `9` files, `embeddedJs: 0`, `evalRefs: 0`,
  `hostJs: 0`, `moduleState: 1` (`src/rules.go:125`, read-only compiled grammar tables & constants).
- `dist/meta.json` attribution (`158,508 B` `-m` emit before facade and Terser):

| Part | Bytes |
| --- | ---: |
| prelude | 41,828 |
| `matrix/marked/src` | 107,244 |
| `runtime` | 4,766 |
| `github.com/gopherjs/gopherjs/js` | 4,437 |
| startup | 233 |

## Quick Node performance

Measured with `tools/bench.mjs --environments node --samples 6` (triplet design, bench variant,
1-minute load average ~4.6 on 10 cores; not a formal benchmark claim — read alongside A/A):

| Case | port p50 (ms) | js p50 (ms) | port/js | paired | A/A |
| --- | ---: | ---: | ---: | ---: | ---: |
| `bench-parse` | 2.86 | 2.30 | 1.25× | 1.24× | 0.992 |
| `bench-gfm` | 2.83 | 2.26 | 1.25× | 1.24× | 0.999 |
