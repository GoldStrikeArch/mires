# Library contract: `marked`

`marked` is a CommonMark and GitHub Flavored Markdown (GFM) compiler to HTML. Ports implement
`marked 18.1.1`'s exact architecture and semantics — block `Lexer` + `Tokenizer`, inline `Lexer` +
`Tokenizer` (including pre-scan delimiter masking for reflinks, escapes, code spans, and HTML),
`Parser`, `Renderer`, `TextRenderer`, and helpers (`escapeHtmlEntities`,
`decodeNumericCharacterReferences`, `cleanUrl`, `splitCells`, `trimTrailingBlankLines`,
`normalizeLabel`, `findClosingBracket`, `expandTabs`) — and must match `marked@18.1.1`
**character-for-character** on every case.

| Field | Value |
| --- | --- |
| Package | `marked@18.1.1`, entry `lib/marked.esm.js` (root `node_modules`) |
| Reference adapter | `reference.mjs`: calls `new Marked({ async: false, silent: false, ...options }).parse(markdown)` or `.parseInline(markdown)` |
| API subset | `Marked#parse(markdown)` (`op: "parse"`) and `Marked#parseInline(markdown)` (`op: "parseInline"`). Options: `gfm` (boolean, default `true`), `breaks` (boolean, default `false`), `pedantic` (boolean, default `false`). Full default `Lexer`, `Tokenizer`, `Parser`, `Renderer`, `TextRenderer`. Left out: custom extensions (`use({ extensions, renderer, tokenizer, walkTokens, hooks })`), `async: true`, `silent: true`, and deprecated options. |
| Input | `{ "op": "parse" \| "parseInline", "markdown": string, "options"?: { "gfm"?: boolean, "breaks"?: boolean, "pedantic"?: boolean } }` |
| Output form | `string` (raw rendered HTML directly, zero framing), synchronous |
| Normalisation | none |
| Parity environments | node, chrome, firefox, safari (unavailable on this machine) |
| Corpus | `corpus.json`, 126 cases, written deterministically by `generate-corpus.mjs` (`--verify` validates all invariants, option mutant-killers and regression expectations) |
| Bench cases | `bench-parse`, `bench-gfm`; `baseIterations` 1; variant `bench-variant.mjs` (k-seeded case-preserving ASCII letter bijection preserving `<...>`, `&...;`, `[x]`/`[X]`, and autolink scheme prefixes) |
| Shared data | `unicode-classes.json`: inclusive code-point ranges read off host `RegExp` for `/[\p{P}\p{S}]/u`, `/[\p{Pi}\p{Ps}"']/u`, `/[\p{L}\p{N}]/u`, and `/\s/u` |
| Caching | Grammar rules compiled at module load, except the six indent-dependent `other.*Regex(indent)` rules (`nextBulletRegex`, `hrRegex`, `fencesBeginRegex`, `headingBeginRegex`, `htmlBeginRegex`, `blockquoteBeginRegex`), which `rules.ts` builds on first use and keeps in a module-level 4-entry cache (`cachedIndentRegex`, indent clamped to 0..3); ports may build those 4 entries eagerly, and may hold immutable character-class tables. No other cross-call state or input memoisation |
| Files | none: neither the adapter nor a port reads files at run time |

## Why this subset

This subset covers the entire synchronous Markdown-to-HTML compilation pipeline of `marked@18.1.1`:
all three grammar modes (CommonMark `normal`, GFM `gfm`/`breaks`, and John Gruber `pedantic`), both
document-level (`parse`) and inline-only (`parseInline`) entry points, all block and inline token
types, link reference definitions, delimiter flanking and masking, HTML escaping and numeric
character reference decoding, URL cleaning, and `TextRenderer` flattening for image `alt` text.
Left out:

- **Extensions & hooks** (`marked.use({ extensions, renderer, tokenizer, walkTokens, hooks })`):
  arbitrary JS callbacks that cannot be represented in a pure JSON corpus and would bypass the
  default lexer/tokenizer/renderer being benchmarked.
- **`async: true`**: exists solely for async hooks/`walkTokens`; synchronous `parse` / `parseInline`
  never returns a Promise.
- **`silent: true`**: catches internal exceptions and wraps them in `<p>An error occurred:</p>`;
  every corpus input is a valid string that completes without throwing.

## Input

Every corpus input is a pure JSON object:

```text
Options (all fields optional):
  {
    "gfm"?:      boolean,   // default true
    "breaks"?:   boolean,   // default false (effective only when gfm === true && pedantic === false)
    "pedantic"?: boolean    // default false (overrides gfm and breaks when true)
  }

Input :=
  { "op": "parse",       "markdown": string, "options"?: Options }
| { "op": "parseInline", "markdown": string, "options"?: Options }
```

## Output

Every call returns a single JS `string`: the exact HTML returned by `new Marked(opts).parse(markdown)`
or `new Marked(opts).parseInline(markdown)`, with zero wrapper or framing.

## Implementation rule on regexes vs. native matchers

`marked@18.1.1` defines its block and inline grammars in `src/rules.ts` (embedded in
`node_modules/marked/lib/marked.esm.js.map`) using JavaScript `RegExp` rules. Port authors using
host `RegExp` (e.g. Scala.js, GopherJS, ReScript, Melange, jsoo, Dart, Kotlin/JS) should note the
ECMAScript regex features used in `src/rules.ts`:

- **`u` flag (`\p{...}` Unicode property escapes):** `other.unicodeAlphaNumeric`, `punctuation`,
  `emStrongLDelim*`, `emStrongRDelimAst*`, `emStrongRDelimUnd*`, `delLDelim`, `delRDelim`,
  `anyPunctuation`, `anyPunctuationOutsideCode`. (All other rules omit the `u` flag — notably
  `inline.link`, `block.def`, `block.html`, `inline.tag`, etc. operate on UTF-16 code units.)
- **Lookbehind & named backreferences:** `blockSkip` uses one negative lookbehind `(?<!`)`
  (when supported by the host engine, which is always true on Node/Chrome/Firefox/Safari) and two
  named backreferences `(?<a>...)...\k<a>` / `(?<b>...)...\k<b>`. Numbered backreferences (`\1`,
  `\2`) appear in `fences`, `block.html`, `blockPedantic.html`, `inline.code`, `inline.del`,
  `anyPunctuationOutsideCode`, and `other.pedanticHrefTitle`.

- **Languages with native JS `RegExp` interop** (ReScript, Melange, js_of_ocaml, Scala.js, Dart,
  Kotlin/JS, GopherJS) may compile and run `marked@18.1.1`'s exact grammar via host `RegExp` or
  implement equivalent native scanners in the target language.
- **Languages without a regex engine in their core** (Rust `no_std` through `mires`, Elm)
  implement `marked@18.1.1`'s exact block and inline rules natively as deterministic scanners in
  the target language, using the code-point tables in `unicode-classes.json` (or equivalent Unicode
  15.0+ tables).

Native scanners must keep three ECMAScript semantics that a line-by-line reading of the rules
easily misses:

- **Line terminators.** No rule uses the `s` flag, so a regex `.` matches any code unit except the
  four ECMA-262 line terminators `\n`, `\r`, U+2028 and U+2029, while `[^\n]` crosses the last
  three. `.` appears in `heading` (all modes), `lheading` (all modes, and the `lheading` lookahead of
  the pedantic `paragraph`), `def`'s `<.*?>`, `gfmTable`'s header and rows (and the GFM
  `paragraph`'s table lookahead), `inline.link`'s `<(?:\\.|[^\n<>\\])+>`, the pedantic
  `inline.link`'s `\((.*?)\)` and `other.pedanticHrefTitle` (also in `other.hrefBrackets` and
  `other.anyLine`, whose inputs never hold a terminator). The three `m`-flag rules (`codeRemoveIndent`, `blockquoteSetextReplace2`,
  `spaceLine`) anchor `^` and `$` at all four terminators. `parse` turns every `\r` into `\n`
  first, but U+2028 and U+2029 reach every block and inline rule, and `parseInline` keeps `\r`.
- **UTF-16 counts.** Every rule outside the `u`-flag list above counts quantifier bounds in UTF-16
  code units: `_boundedBlockLabel`'s `(?:\\[\s\S]|[^\[\]\\]){1,999}` takes
  an astral character, escaped or not, as two items. Every JS `length`, index and `slice` count is
  in code units too, notably `Tokenizer#list`'s `line.slice(indent)`, the `linkLen` truncation, and
  `Tokenizer.ts:809`'s pedantic `<href>` `href.slice(1, -1)` (which drops one UTF-16 code unit from
  each end, splitting a trailing astral surrogate pair so `cleanUrl` returns `null`);
  only `[...str]` spreads (`emStrong` and `del` delimiter lengths) and `expandTabs` count code
  points.
- **Backtracking.** A scanner must find what the backtracking regex finds, not only the greedy
  first try. Cases the corpus pins: `lheadingGfm`'s table lookahead
  ` {0,3}\|?(?:[:\- ]*\|)+[\:\- ]*\n` (the optional `|` backtracks, so a lone `|` or `|-` line
  interrupts), `other.pedanticHrefTitle` (`[^'"]*` backtracks, so the href may end in a quote),
  the pedantic `def`'s `<?([^\s>]+)` (`[a]: <>` has href `<`), `reflinkSearch`'s
  `nolink(?!\()` (`(?:\[\])?` backtracks when `(` follows), optional delimiter escapes in
  `block.def` and `inline.link` titles (`rules.ts:144,472`: `"(?:\\"?|[^"\\])*"`, `'(?:\\'?|[^'\\])*'`,
  `\((?:\\\)?|[^)\\])*\)`, where `\\"?` etc. backtracks to `\` so a trailing backslash closes the
  title), multiple `` ``+(?=\]) `` candidates in `_inlineLabel` / `_boundedInlineLabel`
  (`rules.ts:465,498`: must backtrack across all `` ``] `` candidates in LIFO order when an earlier
  `` ``] `` pairs across `]`), and `inline.link`'s `href` alternative order (`rules.ts:471`: bare
  `[^ \t\n\x00-\x1f]+` precedes `(?=\))`, so `[a]( )<)` matches bare href `)<` and truncates via
  `findClosingBracket`).

Both styles must implement `marked@18.1.1`'s actual Lexer/Tokenizer/Parser/Renderer/TextRenderer
architecture and match `marked@18.1.1` character-for-character on every input in the parity domain.

## Behaviour

Everything below restates `marked@18.1.1` (`node_modules/marked/lib/marked.esm.js` and its original
`src/*.ts` sources in `marked.esm.js.map`, which are the final authority where this is silent).

### Options & rule-set selection (`Lexer.ts`)

```text
if pedantic:
  block = block.pedantic;  inline = inline.pedantic
else if gfm:
  block = block.gfm;       inline = breaks ? inline.breaks : inline.gfm
else:
  block = block.normal;    inline = inline.normal
```

Notice that `breaks: true` has **no effect** when `gfm: false` or `pedantic: true`, and `pedantic: true`
disables the grammar-selected GFM rules (`table`, `del`, `url`, tilde-in-emphasis) as well as fenced
code blocks — **however**, `Tokenizer#list` checks `task: !!this.options.gfm && ...` directly
(`Tokenizer.ts:474`; line numbers here are 1-based lines of `src/Tokenizer.ts` as embedded in
`marked.esm.js.map`), so GFM task-list checkboxes remain **enabled** when `{ gfm: true, pedantic: true }`
and are disabled only when `gfm: false`.

### Entry preprocessing

- **`parse`** (`_Lexer.lex`): first normalises line endings (`src.replace(/\r\n|\r/g, '\n')`), runs
  `blockTokens(src, tokens)`, then drains `inlineQueue` in FIFO order calling
  `inlineTokens(entry.src, entry.tokens)`, and finally renders via `_Parser.parse(tokens)`.
- **`parseInline`** (`_Lexer.lexInline`): does **not** normalise `\r\n` / `\r`; runs
  `inlineTokens(src, [])` directly (with an empty `links` table) and renders via
  `_Parser.parseInline(tokens)`.

### Helpers (`helpers.ts`)

1. **JS whitespace (`\s`, `.trim()`, `.trimStart()`, `.trimEnd()`)**: the 25 ECMA-262 whitespace
   and line-terminator code points: `U+0009..000D`, `U+0020`, `U+00A0`, `U+1680`, `U+2000..200A`,
   `U+2028`, `U+2029`, `U+202F`, `U+205F`, `U+3000`, `U+FEFF`.
2. **`escapeHtmlEntities(html, encode = false)`**:
   - Replacements: `&` → `&amp;`, `<` → `&lt;`, `>` → `&gt;`, `"` → `&quot;`, `'` → `&#39;`.
   - When `encode === true`: replaces every `[&<>"']`.
   - When `encode === false`: replaces `[<>"']`, and replaces `&` only when **not** followed by
     `(#\d{1,7}|#[Xx][a-fA-F0-9]{1,6}|\w+);` (ASCII `\w` = `[A-Za-z0-9_]`).
3. **`decodeNumericCharacterReferences(text)`**:
   - If `text` contains no `/&#(?:(\d{1,7})|[Xx]([A-Fa-f0-9]{1,6}));/`, returns `text` unchanged.
   - Otherwise scans left-to-right for numeric refs `&#dec;` (1–7 digits) / `&#xhex;` (1–6 hex digits)
     and bare `&` not starting `(#\d{1,7}|#[Xx][a-fA-F0-9]{1,6}|\w+);`:
     - Bare `&` → `&amp;` (prevents decoded chars from merging with a preceding `&` into an entity).
     - Numeric ref with `code === 0 || code > 0x10FFFF || (0xD800 <= code <= 0xDFFF)` → `U+FFFD`.
     - Numeric ref with `code === 38` → `&amp;`.
     - Otherwise → Unicode character `String.fromCodePoint(code)`.
4. **`cleanUrl(href)`**: `encodeURI(href).replace(/%25/g, '%')`, catching `URIError` (e.g. on a
   lone UTF-16 surrogate produced by `Tokenizer.ts:809`) and returning `null`. For well-formed
   UTF-8/UTF-16 strings this keeps ASCII bytes in `!#$%&'()*+,-./0123456789:;=?@ABCDEFGHIJKLMNOPQRSTUVWXYZ_abcdefghijklmnopqrstuvwxyz~`
   verbatim and percent-encodes every other UTF-8 byte as uppercase `%XX`.
5. **`splitCells(tableRow, count?)`**: inserts `' '` before every unescaped `|` (even number of
   preceding `\`), splits on `' |'`, drops an empty/whitespace-only leading or trailing cell,
   truncates or pads with `""` to `count` if `count` is positive, then trims each cell and replaces
   `\|` with `|`.
6. **`trimTrailingBlankLines(str)`**: splits on `'\n'`, finds the last line index `end` not matching
   `/^[ \t]*$/`. If `lines.length - end <= 2` (0 or 1 trailing blank line), returns `str` unchanged;
   otherwise joins `lines[0..=end]` (inclusive) with `'\n'`.
7. **`normalizeLabel(label)`**: `label.trim().toLowerCase().toUpperCase().toLowerCase()` (full
   Unicode case folding, e.g. `ẞ` → `ß` → `SS` → `ss`).
8. **`findClosingBracket(str, "()")`**: returns `-1` if neither `(` nor `)` appears; otherwise scans
   skipping `\x` escapes, incrementing depth on `(` and decrementing on `)`. Returns the index of the
   first unmatched `)`, `-2` if unmatched `(` remains at end, or `-1` if balanced.
9. **`expandTabs(line, indent = 0)`**: expands each `'\t'` at column `col` (counting Unicode scalar
   values starting at `indent`) to `4 - (col % 4)` spaces.

### Block Lexer & Tokenizer (`Lexer.ts`, `Tokenizer.ts`, `rules.ts`)

When `options.pedantic` is true, each `blockTokens` entry first runs
`src.replace(/\t/g, '    ').replace(/^ +$/gm, '')`.

In the block loop, rules are tried in this exact order:

1. **`space`** (`/^(?:[ \t]*(?:\n|$))+/`): if `raw.length === 1` (`"\n"`) and a prior token exists,
   appends `'\n'` to `lastToken.raw` instead of pushing a `space` token.
2. **`code`** (indented, `/^((?: {4}| {0,3}\t)[^\n]+(?:\n(?:[ \t]*(?:\n|$))*)?)+/`):
   `raw = pedantic ? cap[0] : trimTrailingBlankLines(cap[0])`; strips `/^(?: {0,3}\t| {1,4})/gm`
   to form `text`. If `lastToken` is `paragraph` or `text`, appends to `lastToken` (setting the
   last `inlineQueue` entry's `src` to the merged `lastToken.text`) because indented code cannot
   interrupt a paragraph.
3. **`fences`** (disabled in `pedantic`): 0–3 leading spaces, `≥3` backticks (info string has no
   backticks) or `≥3` tildes, info string, body lines until ` {0,3}\1[~`]* *(?=\n|$)` or EOF.
   - `indentCodeCompensation`: **only** when `raw` starts with `/^(\s+)(?:```)/` (backtick fences,
     NOT tilde fences!), removes up to `indent.length` leading whitespace chars from each body line.
   - `lang`: trimmed info string with `anyPunctuation` (`\\([\p{P}\p{S}])`) unescaped.
4. **`heading`** (ATX): normal/GFM `/^ {0,3}(#{1,6})(?=\s|$)(.*)(?:\n+|$)/`; pedantic
   `/^(#{1,6})(.*)(?:\n+|$)/`. Strips trailing `#`s (only when preceded by `[ \t]` or empty unless
   `pedantic`). `raw` has trailing `\n`s trimmed via `rtrim`.
5. **`hr`**: `/^ {0,3}((?:-[\t ]*){3,}|(?:_[ \t]*){3,}|(?:\*[ \t]*){3,})(?:\n+|$)/`; `raw` rtrimmed of `\n`.
6. **`blockquote`**: matches leading ` {0,3}> ?` lines plus lazy `blockquoteParagraph` continuations;
   iteratively strips `blockquoteSetextReplace` (`\n {0,3}((?:=+|-+) *)(?=\n|$)` → `\n    $1`) and
   `blockquoteSetextReplace2` (`/^ {0,3}>[ \t]?/gm`), recursively parses with `top = true` and
   `lastParagraphClipped = true`, stops continuation after `code`/`heading`/`hr`/`html`, and merges
   continuations into nested `blockquote` or `list` tokens.
7. **`list`**: matches bullet (`[*+-]` or `\d{1,9}[.)]`), groups items sharing the same marker
   (in `pedantic`, all unordered `[*+-]` merge), stops on `hr`, computes item indentation (with
   `expandTabs` and `> 4` indented-code rule), ends items on interrupting blocks (`fences`, `heading`,
   `html` case-insensitive, `blockquote`, `nextBullet`, `hr` at `clamp(indent - 1, 0, 3)` spaces),
   supports lazy continuation, trims the final item's trailing whitespace, tokenises items with
   `top = false`, determines `list.loose` (inter-item double blank line or intra-item `space`
   matching `/\n.*\n/`), extracts GFM task checkboxes (`/^\[[ xX]\] +\S/`, placing `checkbox` inside
   the first `paragraph`/`text` when `loose` or at `item.tokens[0]` when tight, and mutating the
   matching `inlineQueue` entry), and promotes `text` tokens to `paragraph` when `list.loose`.
8. **`html`**: normal/GFM implements all 7 CommonMark HTML block types (case-insensitive) +
   `trimTrailingBlankLines`; pedantic uses its case-sensitive block-tag/comment grammar excluding
   inline tags.
9. **`def`**: link reference definition (`[label]: dest title?`); rejects bare destinations with
   unbalanced `()`. If `lastToken` is `paragraph` or `text`, appends `raw` to `lastToken`;
   otherwise registers `tokens.links[tag]` only if `tag` is not yet defined (first definition wins).
   Unescapes `anyPunctuation` in `href` and `title`.
10. **`table`** (GFM only): header row, delimiter row (must contain `:` or `|`, same column count
    as header), and body rows up to blank line or interrupting block (`hr`, `heading`, `blockquote`,
    `code`, `fences`, `list`, `html`). Splits cells with `splitCells` and parses each cell inline.
11. **`lheading`** (Setext): `=` → `h1`, `-` → `h2`. Normal/GFM supports multiline text not
    interrupted by blank line/list/fences/blockquote/heading/hr/html (and GFM table); pedantic is
    single-line only.
12. **`paragraph`** (when `state.top === true`): consumes lines until blank line (`[ \t]+\n`) or an
    interrupting block (note: paragraph HTML interrupt is case-sensitive types 1–6; in GFM, `gfmTable`
    regex interrupts paragraphs even if column counts mismatch). Merges with prior `paragraph` when
    `lastParagraphClipped` is true.
13. **`text`** (fallback when `state.top === false`, e.g. tight list items): consumes one line and
    merges consecutive `text` tokens.

### Inline Lexer & Tokenizer (`Lexer.ts`, `Tokenizer.ts`, `rules.ts`)

Each `inlineTokens(src, tokens)` call:
1. Computes `state.linkParenPossible = prev && src.includes(')')` (restored on exit).
2. Constructs `maskedSrc` (same length as `src`, so end-relative offsets stay aligned) in 3 passes:
   - Masks defined reflinks via `reflinkSearch` (preserving nested links inside non-image candidates
     detected by `linkInText`; note `match0.lastIndexOf('[')` is used for the ref lookup, so
     collapsed `[a*b][]` looks up `""` and stays unmasked, matching `marked@18.1.1`).
   - Runs `anyPunctuationOutsideCode`: replaces `\` + `[\p{P}\p{S}]` outside code spans with `'+'`s,
     and replaces inner `` ` `` inside code spans with `'+'`.
   - Runs `blockSkip` (`link | (?<!`)code | html`), replacing each match with `'[' + 'a'*(len-2) + ']'`.
3. Loops over `src` maintaining `prevChar` (last UTF-16 code unit of the previous `inlineText` token
   unless `'_'`; thus an astral character leaves a lone low surrogate that matches neither
   `unicodeAlphaNumeric` nor `punctuation`) and `keepPrevChar`, trying rules in exact order:
   1. `escape` (32 ASCII punctuation chars)
   2. `tag` (inline comment, close tag, open tag, PI, declaration, CDATA; updates `inLink` on
      `/^<a /i` / `/^<\/a>/i` and `inRawBlock` on `/^<(pre|code|kbd|script)(\s|>)/i` / closing)
   3. `link` (skipped if `!linkParenPossible`; 2-level nested `[]` label; `isLabelEndInsideToken`
      check when `!pedantic`; angle-bracket or balanced-paren href; `outputLink` rejects nested
      links inside non-image links via `state.linkEmitted`)
   4. `reflink` / `nolink` (full `[text][ref]`, collapsed `[ref][]`, shortcut `[ref]`; undefined
      label emits single-char `text` token without setting `keepPrevChar`)
   5. `emStrong` (left/right flanking rules for `*` and `_` in normal/GFM/pedantic, CommonMark
      rule 9–10 multiple-of-3, `midRun`, orphan-in-strong skip, excess-delimiter trimming)
   6. `codespan` (matching backtick run length, `\n` → `' '`, single outer space strip when both ends
      are space and content is not all spaces)
   7. `br` (`( {2,}|\\)\n(?!\s*$)[ \t]*`, or `( *|\\)\n(?!\s*$)[ \t]*` under `breaks`)
   8. `del` (GFM/breaks only: `~` or `~~` matching delimiter lengths, never `~~~`)
   9. `autolink` (`<scheme:uri>` or `<email>`, sets `autolink: true`)
   10. `url` (GFM/breaks only, skipped when `state.inLink`: `mailto:`/`xmpp:`, `http(s)://`/`ftp://`/`www.`,
       bare email, with iterative `_backpedal` trailing punctuation/paren/entity trimming)
   11. `inlineText` (stops at inline triggers; when `!inRawBlock`, decodes numeric character
       references via `decodeNumericCharacterReferences`; merges adjacent `text` tokens preserving
       the first token's `escaped` flag).

### Renderer & TextRenderer (`Renderer.ts`, `TextRenderer.ts`)

- `code`: `<pre><code[ class="language-LANG"]>CODE</code></pre>\n`, where `LANG` is the first
  non-whitespace run of `lang` escaped with `escapeHtmlEntities(lang, false)`, and `CODE` is `""`
  if `text === ""` else `text.replace(/\n$/, '') + '\n'` escaped with `escapeHtmlEntities(code, true)`.
- `heading`: `<h{depth}>...</h{depth}>\n`. `hr`: `<hr>\n`. `blockquote`: `<blockquote>\n...</blockquote>\n`.
- `list`: `<ul|ol[ start="N"]>\n...</ul>\n` (`start` attribute emitted iff `ordered && start !== 1`).
- `listitem`: `<li>...</li>\n`. `checkbox`: `<input [checked="" ]disabled="" type="checkbox"> `.
- `paragraph`: `<p>...</p>\n`. `table`: `<table>\n<thead>\n<tr>\n...</tr>\n</thead>\n[<tbody>...</tbody>]</table>\n`
  (cells are `<th|td[ align="..."]>...</th|td>\n`; `<tbody>` omitted when `rows.length === 0`).
- `strong`, `em`, `del`: `<strong>`, `<em>`, `<del>`. `codespan`: `<code>${escapeHtmlEntities(text, true)}</code>`.
- `br`: `<br>`. `html`: raw `text` passthrough. `space`, `def`: `""`.
- `link`: `parsedText = autolink ? escapeHtmlEntities(text, true) : parseInline(tokens)`;
  `cleanHref = cleanUrl(href)`; if `cleanHref === null`, returns `parsedText`; otherwise emits
  `<a href="${escapeHtmlEntities(cleanHref, autolink)}"[ title="${escapeHtmlEntities(title)}"]>${parsedText}</a>` (the `title` attribute is omitted when `title` is absent or empty `""`).
- `image`: flattens `tokens` through `TextRenderer` (which returns raw `.text` for `strong`/`em`/`codespan`/`del`/`html`/`text`/`link`/`image`, `""` for `br`, `.raw` for `checkbox`), computes `cleanHref = cleanUrl(href)`; if `cleanHref === null`, returns `escapeHtmlEntities(alt)`; otherwise emits
  `<img src="${escapeHtmlEntities(cleanHref)}" alt="${escapeHtmlEntities(alt)}"[ title="${escapeHtmlEntities(title)}"]>` (likewise omitting `title="..."` when `title` is absent or empty `""`).
- `text`: if `tokens` present, `parseInline(tokens)`; else if `escaped`, raw `text`; else `escapeHtmlEntities(text, false)`.

## Parity domain

The following inputs are excluded from the contract (no corpus case or bench variant produces them):

- **Lone UTF-16 surrogates in inputs or output HTML** (including upstream `marked` extra-`)` link truncation splitting an
  astral surrogate pair, e.g. `[x](  🚀))`, where `Tokenizer.ts:785`'s UTF-16
  `cap[0].substring(0, linkLen)` (`linkLen` from line 783) slices between high and low surrogates and leaves a lone surrogate in the remainder): all corpus
  `markdown` strings are well-formed UTF-16 and stay well-formed UTF-16 across UTF-16 `linkLen`
  truncation (BMP non-ASCII cases like `[x](  é))` and `[x](    日本語))` ARE in the parity domain).
  By contrast, `Tokenizer.ts:809`'s pedantic `<href>` `href.slice(1, -1)` on well-formed UTF-16
  input ending in an astral scalar (e.g. `[a](<😀 "t">)` or `![a](<😀 "t">)` with `pedantic: true`)
  splits the surrogate pair internally, causing `cleanUrl(href)` to catch `URIError` and return
  `null`, so `Renderer.link`/`Renderer.image` emit plain/escaped text (`<p>a</p>\n`) with no lone
  surrogate in the output; those inputs ARE in the parity domain and ports must reproduce
  `Tokenizer.ts:809`'s UTF-16 `slice(1, -1)` and `cleanUrl`'s `null` fallback.
- **Unicode 16.0 / 17.0-only punctuation/symbol/letter code points**: no corpus case uses code
  points whose `\p{P}`, `\p{S}`, `\p{Pi}`, `\p{Ps}`, `\p{L}`, `\p{N}`, or case-folding properties
  changed after Unicode 15.0, so Unicode 15.0–17.0 engines and native tables agree identically.
- **Non-string `markdown` inputs**: `markdown` is always a JSON string.
- **Nesting deeper than 100 levels**: block nesting and inline nesting are each bounded at 100
  levels.
  - **Blocks.** Lists and blockquotes are nested at most 100 levels deep, the two kinds counted
    together. marked recurses once per level (`blockTokens` → `list` or `blockquote` →
    `blockTokens`), and so do the ports. marked itself overflows Node's default stack at about 2,490
    nested lists. The compiled ports overflow earlier: about 1,000 for most, about 620 for Melange
    and about 500 for Elm. The Rust port overflows at about 200–280, because its mires-generated
    frames are large (`MIRES-BUGS.md`, `PERF-005`).
  - **Inlines.** Inside one block, emphasis, strong, strikethrough and link or image text are nested
    at most 100 levels deep, all kinds counted together. marked lexes each level with a recursive
    `inlineTokens` (from `emStrong`, `del` and the link and image builder) and renders it with a
    recursive `parseInline`, and so do the ports. In Node, marked overflows at about 2,421 nested
    emphasis runs. The ports diverge earlier: Elm from about 484, Rust 968, Melange 1,210, Kotlin
    1,452, Dart 1,694 and GopherJS 1,845. ReScript, js_of_ocaml and Scala.js hold to at least
    1,936 (cleanup A's measurement, `build/library-matrix/review-20261120/cleanup-a/`). Links do
    not nest deeply. A link label admits one level of brackets, so 16,000 nested `[` stay within
    every port's stack.
  - **Counted separately.** marked lexes inline content from its queue after the block pass, so
    only the renderer stacks the two recursions. Every port renders 100 inline levels inside 100
    lists.
  - **Corpus.** It has one case at exactly 100 levels for lists, one for blockquotes and one for
    inline containers, and one with 100 inline levels inside 100 lists.

## Bench cases and variants

- **`bench-parse`**: 51,788-character mixed CommonMark + GFM technical handbook (32 sections with
  ATX & setext headings, rich inline prose, nested blockquotes with lazy continuations, backtick &
  tilde fenced code blocks, indented code, nested ordered/unordered & task lists, aligned GFM tables
  with escaped pipes, inline & reference links, images, autolinks, HTML blocks & inline HTML,
  entities, escapes, and non-ASCII text) under `{ gfm: true, breaks: false, pedantic: false }`
  (renders 84,988 bytes of HTML, ~2.1 ms per call in Node).
- **`bench-gfm`**: 45,665-character GFM-heavy issue tracker & tables document (44 issues with dense
  GFM tables, task checklists, `http`/`https`/`ftp`/`www`/`mailto`/email autolinks with `_backpedal`
  punctuation & parens, strikethrough, inline code, and single-newline line breaks) under
  `{ gfm: true, breaks: true, pedantic: false }` (renders 115,858 bytes of HTML, ~2.3 ms per call in
  Node).

**Bench variant** (`bench-variant.mjs`, registry `bench.variant`): applies a `k`-seeded
case-preserving bijection on ASCII letters across the entire document while leaving `<...>`,
`&...;`, `[x]`/`[X]`, and autolink scheme prefixes untouched. All 50 variants (`k = 1..50`) produce
50/50 distinct inputs and 50/50 distinct outputs with **0-byte length drift** and every 256-char
window modified.

## Corpus

126 deterministic cases generated by `generate-corpus.mjs`:

| Group | Cases | What they exercise |
| --- | --- | --- |
| Basics & `parseInline` (4) | `empty`, `whitespace-only`, `crlf-and-cr`, `parse-inline-basic` | empty/blank input, CRLF/CR normalisation, `parseInline` vs `parse` |
| Options matrix (9) | `opts-gfm-true`, `opts-gfm-false`, `opts-breaks-true`, `opts-breaks-false`, `opts-gfm-false-breaks-true`, `opts-pedantic-true`, `opts-pedantic-false`, `opts-pedantic-gfm-true-task`, `opts-pedantic-gfm-false-task` | mutant-killing proof that `gfm`, `breaks`, and `pedantic` each change output, follow `Lexer` precedence, and preserve task lists under `{ gfm: true, pedantic: true }` |
| Headings & HR (5) | `headings-atx-all-levels`, `headings-setext`, `headings-inline-content`, `headings-atx-pedantic`, `hr-variants` | ATX h1–h6 & closing `#`, setext h1/h2 & precedence, pedantic headings & `# ` paragraph interrupts, thematic breaks |
| Code blocks (7) | `code-indented-basic`, `code-indented-no-interrupt`, `code-indented-pedantic`, `code-fenced-backticks`, `code-fenced-tildes`, `code-fenced-indent-compensation`, `code-fenced-info-and-fallback` | indented code & trailing-blank trimming (incl. tight-list 6-space code merge), backtick/tilde fences, `indentCodeCompensation` (backtick-only), info string unescaping/escaping, fallback |
| Blockquotes (5) | `blockquote-basic-nested`, `blockquote-lazy-continuation`, `blockquote-nested-continuation`, `blockquote-list-continuation`, `blockquote-interrupts-paragraph` | nested quotes, lazy continuations, `blockquoteSetextReplace` (`===` & `---`), list & nested quote continuations |
| Lists & Tasks (9) | `list-unordered-markers`, `list-ordered-start`, `list-tight-vs-loose`, `list-nested-and-indent`, `list-lazy-and-interrupts`, `list-task-tight`, `list-task-loose`, `list-task-non-text-first`, `list-pedantic` | bullet splitting, ordered start/delimiters, tight vs loose, tab expansion & nesting, GFM task checkboxes (tight, loose, setext non-text clear), pedantic lists |
| GFM Tables (6) | `table-alignments`, `table-escaped-pipes-and-code`, `table-header-only`, `table-mismatched-rows`, `table-invalid-fallback`, `table-interrupted-by-blocks` | alignments, `splitCells` escaped pipes, header-only (no `<tbody>`), row pad/truncate, invalid fallback & paragraph interruption, block interrupts |
| HTML (6) | `html-blocks-types-1-to-5`, `html-blocks-type-6-and-7`, `html-block-trailing-blanks`, `html-inline-tags-and-raw`, `html-inline-comments-pi-cdata`, `html-pedantic` | all 7 CommonMark HTML block types, case-sensitive paragraph interrupt, `inLink`/`inRawBlock` inline tag state & raw text merging, pedantic HTML vs inline `<span>` |
| Defs, Links, Images & Autolinks (10) | `def-normalization-and-precedence`, `def-destinations-and-titles`, `links-inline-variants`, `links-extra-paren-unicode`, `links-ref-full-collapsed-shortcut`, `links-no-nesting-and-images`, `links-label-inside-tag-or-code`, `images-text-renderer`, `links-pedantic`, `autolinks-commonmark` | Unicode case-folded labels, first-def wins, tight-list def merge, `<href>` backtracking & unbalanced parens, UTF-16 extra-`)` truncation, `cleanUrl`, nested link suppression, `isLabelEndInsideToken`, `TextRenderer` alt flattening, pedantic links/defs, `<autolink>` |
| GFM Autolinks (3) | `gfm-autolinks-urls`, `gfm-autolinks-backpedal`, `gfm-autolinks-disabled-in-normal` | `http`/`https`/`ftp`/`www`/`mailto`/`xmpp`/email autolinks, `_backpedal` trimming, `inLink` & `gfm: false` |
| Emphasis, Strong, Del & Codespans (8) | `em-strong-asterisk-flanking`, `em-strong-underscore-flanking`, `em-strong-unicode-punctuation`, `em-strong-masking`, `em-strong-gfm-tilde`, `em-strong-pedantic`, `gfm-strikethrough`, `codespans-edge-cases` | `*`/`_` flanking, rule 9–10, isolated `midRun`, `foo___bar___` prevChar, orphan skip, Unicode `\p{P}`/`\p{S}` & astral `prevChar`, pre-scan masking (incl. escapes inside span), GFM `~` in emphasis, pedantic `_openQuote`, `~`/`~~` del, code spans |
| Escapes, Breaks, Entities, Unicode (6) | `backslash-escapes`, `hard-breaks`, `html-entities-and-ncr`, `unicode-cjk-emoji-rtl`, `parse-inline-options`, `parse-inline-trailing-br-ws` | 32 ASCII escapes, hard breaks & `(?!\s*$)` trailing-ws lookahead, NCR decoding + bare `&` + raw block, CJK/RTL/emoji/Unicode whitespace, `parseInline` + options |
| Review regressions: line terminators (14) | `regress-dot-heading-u2028`, `regress-dot-heading-u2029`, `regress-dot-heading-cr`, `regress-dot-pedantic-heading-u2028`, `regress-dot-lheading-u2029`, `regress-dot-pedantic-lheading-lookahead`, `regress-dot-table-header-u2028`, `regress-dot-table-row-u2028`, `regress-dot-def-angle-u2028`, `regress-dot-pedantic-inline-link-cr`, `regress-dot-link-angle-escape-cr`, `regress-mflag-code-indent`, `regress-mflag-blockquote-marker`, `regress-mflag-pedantic-space-line` | regex `.` stops at `\r`, U+2028 and U+2029 in heading, lheading, table, def, link and pedantic link; `\r` normalised by `parse`, kept by `parseInline`; `m`-flag `^`/`$` at every terminator in code-indent removal, blockquote marker stripping and pedantic space lines |
| Review regressions: UTF-16 counts (5) | `regress-utf16-list-lazy-astral`, `regress-utf16-list-lazy-astral-tab`, `regress-utf16-list-lazy-heading`, `regress-utf16-bounded-label`, `regress-utf16-bounded-label-edge` | `Tokenizer#list`'s `line.slice(indent)` in code units after an astral character; `_boundedBlockLabel` `{1,999}` counting an astral character as two units (1,202 and 1,000 units: unmasked) |
| Review regressions: backtracking and rule sources (10) | `regress-lheading-table-pipe-dash`, `regress-lheading-table-lone-pipe`, `regress-lheading-table-lone-pipe-hr`, `regress-pedantic-href-title-backtrack`, `regress-pedantic-inline-href-title-backtrack`, `regress-pedantic-href-title-mirror`, `regress-pedantic-def-empty-angle`, `regress-pedantic-def-angle-title`, `regress-nolink-empty-ref-paren`, `regress-html-begin-ul` | lheadingGfm's optional `\|` backtracking, `pedanticHrefTitle` and pedantic `def` `<?` backtracking, `reflinkSearch` `nolink(?!\()` backtracking `[]`, and `htmlBeginRegex`'s last `_tag` entry (`ul`) ending a list item |
| Review regressions: stack depth (6) | `regress-size-atx-headings`, `regress-size-emphasis`, `regress-depth-list-100`, `regress-depth-blockquote-100`, `regress-depth-inline-100`, `regress-depth-list-100-inline-100` | 5,000 headings and 5,000 emphasis runs (20 KB each): block and inline loops in constant stack; 100 nested lists, 100 nested blockquotes and 100 nested inline containers (link text around em, strong and del), the parity domain's nesting bounds; 100 inline levels inside 100 lists, the two bounds stacked in the renderer |
| Review regressions: pedantic `<href>`, title escapes, label & href backtracking (11) | `regress-pedantic-angle-href-bmp`, `regress-pedantic-angle-href-astral`, `regress-def-title-escape-backtrack`, `regress-link-title-escape-double`, `regress-link-title-escape-single`, `regress-link-title-escape-paren`, `regress-inline-label-multi-backtick-1`, `regress-inline-label-multi-backtick-2`, `regress-inline-label-multi-backtick-3`, `regress-inline-label-multi-backtick-ref`, `regress-link-href-rparen-alt` | pedantic `<href>` UTF-16 `slice(1,-1)` on BMP non-ASCII and astral surrogate-splitting (`cleanUrl` null fallback), optional-escape backtracking in `block.def` and `inline.link` titles, LIFO multi-candidate `` ``] `` backtracking in `_inlineLabel`, and `inline.link` bare `href` priority before `(?=\))` |
| Bench (2) | `bench-parse`, `bench-gfm` | 52 KB mixed handbook (default options) and 46 KB GFM tracker (`breaks: true`) |
