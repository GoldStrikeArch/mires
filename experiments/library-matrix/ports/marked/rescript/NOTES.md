# marked 18.1.1: ReScript port

A faithful port of the synchronous CommonMark, GFM, and pedantic Markdown compiler defined in
`libraries/marked/CONTRACT.md` to ReScript 12.3.0: the same grammar rule sets (`normal`, `gfm`,
`breaks`, `pedantic`), block and inline `Lexer` + `Tokenizer` (including pre-scan delimiter
masking), `Parser`, `Renderer`, `TextRenderer`, and helpers as `node_modules/marked/lib/marked.esm.js`
(and its embedded `src/*.ts` sources), with zero `%raw`/`%%raw`, zero `Obj.magic`, zero handwritten
JS, zero `@rescript/runtime` imports, and zero cross-call mutable state. It reaches exact
character-for-character parity on all 80 corpus cases in Node, Chrome, and Firefox (both minified
and readable artifacts), passes a 225/225 regex `.source`/`.flags` equality check against
`marked@18.1.1`, and passes 2,824/2,824 adversarial and randomized differential cases across all 8
`(gfm, breaks, pedantic)` option combinations and both `parse`/`parseInline`.

| File | What it is |
| --- | --- |
| `src/Rules.res` | Zero-cost `RegExp` interop bindings (`execNull`, `groupOpt`, `groupUnsafe`, typed `replace` callbacks), exact `edit` regex builder, `supportsLookbehind` probe, eagerly prebuilt 4-entry indent tables (`nextBullet`, `hr`, `fencesBegin`, `headingBegin`, `htmlBegin`, `blockquoteBegin`), and immutable compiled rule tables (`blockNormal`, `blockGfm`, `blockPedantic`, `inlineNormal`, `inlineGfm`, `inlineBreaks`, `inlinePedantic`) |
| `src/Helpers.res` | `escapeHtmlEntities` (switch-based replacement), `decodeNumericCharacterReferences`, `cleanUrl`, `splitCells`, `rtrim`, `trimTrailingBlankLines`, `normalizeLabel`, `findClosingBracket`, zero-iterator UTF-16 surrogate-aware `expandTabs`, `firstLineOf`, `codePointLength`, and `firstCodePointUtf16Len` |
| `src/Tokens.res` | `options`, `linkDef`, `tableCell`, `listItem`, split `blockToken` / `inlineToken` variants, `inlineQueueEntry`, and per-call `lexer` state record |
| `src/Tokenizer.res` | All block tokenizers (`space`, `code`, `fences`, `heading`, `hr`, `blockquote` with nested quote/list continuations, `list` with tabs/interrupts/tight-vs-loose/GFM task checkboxes, `html`, `def`, `table`, `lheading`, `paragraph`, `text`) and inline tokenizers (`escape`, `tag`, `link`, `reflink`, `emStrong`, `codespan`, `br`, `del`, `autolink`, `url`, `inlineText`, plus `outputLink`, `indentCodeCompensation`, `isLabelEndInsideToken`) |
| `src/Lexer.res` | `lex` (CRLF/CR normalisation, `blockTokens`, FIFO `inlineQueue` drain), `lexInline`, `linkInText`, 3-pass `maskedSrc` pre-scan (`maskReflink`, `maskAnyPunctuationOutsideCode`, `maskBlockSkip`), `inlineTokens` with `linkParenPossible` scoping |
| `src/TextRenderer.res` | Plain-text inline token flattener for image `alt` attributes |
| `src/Renderer.res` | HTML emitters for every block and inline token type |
| `src/Parser.res` | Recursive `parse` and `parseInline` dispatchers over `blockToken` and `inlineToken` |
| `src/Index.res`, `src/Index.resi` | Boundary matching `libraries/marked/reference.mjs`; `run` is the module's only export and throws `TypeError` on unknown `op` |

## Decisions

- **Immutable module-level grammar tables and zero cross-call state.** Like `marked` itself, all
  grammar regexes are compiled once at module load via `edit` into immutable records; the four
  possible indent-clamped variants (`0..3`) for each list-interrupt helper regex are constructed
  eagerly at module load instead of lazily cached, and `escapeHtmlEntities` uses a `switch` rather
  than a module-level dictionary (`module state` lint: `0`). Every shared `/g` regex whose
  `lastIndex` is read or advanced via `exec` (`emStrongRDelimAst`, `emStrongRDelimUnd`, `delRDelim`,
  `blockSkip`, `reflinkSearch`) resets or saves/restores `lastIndex` around every scan and recursion
  so no stale `lastIndex` ever survives across calls. Disabled rules (`table` in normal/pedantic,
  `fences` in pedantic, `del`/`url`/`_backpedal` in normal/pedantic) are typed as `option<RegExp.t>`
  (`None`), avoiding noop `exec` calls altogether.
- **Prototype-safe link table.** Per-call `lexer.links` uses a JS `Map.t<string, linkDef>` rather
  than ReScript's `dict` (plain `{}` inherits `Object.prototype`), matching `marked`'s
  `Object.create(null)` + `Object.hasOwn` semantics for labels such as `toString`, `__proto__`, and
  `constructor`, and allowing `inlineTokens` to skip `reflinkSearch` in O(1) when `links` is empty.
- **Zero-iterator Unicode helpers and zero-runtime externals.** Replacing JS `[...s].length`,
  `[...s][0].length`, `for (const ch of line)` (`expandTabs`), and `split('\n', 1)[0]` with
  surrogate-pair-aware UTF-16 scans (`codePointLength`, `firstCodePointUtf16Len`, `expandTabs`,
  `firstLineOf`) avoids iterator and temporary-array allocations in hot paths while keeping exact
  code-point semantics. Typing `RegExp.exec` via `Null.t<RegExp.Result.t>` and capture groups via
  `option<string>` compiles to direct `!== null` / `!== undefined` checks with `0 B` of
  `@rescript/runtime` pulled into the bundle.
- **Separate `blockToken` and `inlineToken` variants.** Splitting block and inline tokens gives
  exhaustive pattern matching across `Parser.parse`, `Parser.parseInline`, and `TextRenderer`
  without dead fallback branches; `Paragraph` and `Text` share a mutable `textBlock` record so
  loose-list promotion (`Text(tb)` -> `Paragraph(tb)`) shares the exact same `tokens` array queued
  in `inlineQueue`.
- **Boundary.** `input` is typed as a ReScript record `{op: string, markdown: string, options?: optionsJson}`
  matching the JSON input zero-copy; omitted options default to `{ gfm: true, breaks: false, pedantic: false }`,
  unknown `op` throws `TypeError`, and the rendered HTML JS string returns directly.

## ReScript 12.3.0 notes

- Native regex literals (`/…/flags`, including `/u` Unicode property escapes `\p{P}`, `\p{S}`,
  `\p{L}`, `\p{N}`, `\p{Pi}`, `\p{Ps}`) compile straight to JS regex literals without escaping
  overhead; `blockSkip`'s named backreferences and lookbehind compile cleanly via `RegExp.fromString`.
- Precedence reminders: `:=` binds tighter than `? :` (parenthesize ternary RHS values), unary `-`
  binds tighter than `->` (`-(src->String.length)`), and field access `.src` binds tighter than `->`
  (`(arr->Array.getUnsafe(i)).src = ...`).
- Inspecting `dist/index.js` confirmed zero `@rescript/runtime` imports (`0 B`), zero boxed
  `.contents` refs, and zero per-iteration IIFEs in `while` loops.

## Size and quick Node performance

- **Bundle size (`dist/index.min.js`):** `38,331 B` raw, `12,324 B` gzip-9, `11,094 B` brotli-11
  (JS reference: `46,922 B` raw, `13,794 B` gzip-9, `12,495 B` brotli-11).
- **Rollup attribution (before Terser, `dist/meta.json`):** `90,128 B` port code across 9 modules
  (`Tokenizer` 31.2 KB, `Rules` 22.4 KB, `Lexer` 22.3 KB, `Helpers` 5.9 KB, `Renderer` 4.0 KB,
  `Parser` 2.4 KB, `Index` 0.8 KB, `Tokens` 0.6 KB, `TextRenderer` 0.6 KB) plus `0 B`
  `@rescript/runtime`.
- **Source lints:** `0` embedded JS, `0` eval refs, `0` host JS, `0` module state.
- **Reproducibility:** verified byte-identical across consecutive `--clean` builds.
- **Quick Node perf (`bench.mjs --environments node --samples 6`, load ~4.25):**
  - `bench-parse`: `2.13 ms` vs `2.26 ms` JS (`port/js` `0.95`, paired `0.92`, A/A `1.001`).
  - `bench-gfm`: `2.18 ms` vs `2.37 ms` JS (`port/js` `0.92`, paired `0.94`, A/A `0.998`).
