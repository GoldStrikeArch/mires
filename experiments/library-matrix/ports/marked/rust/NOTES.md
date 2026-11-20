# marked 18.1.1: Rust port (mires)

A port of the `marked@18.1.1` synchronous Markdown-to-HTML subset defined in
`libraries/marked/CONTRACT.md`: the block/inline `Lexer`, `Tokenizer`, `Parser`, `Renderer`,
`TextRenderer`, helpers, and CommonMark / GFM / pedantic rule precedence of
`node_modules/marked/lib/marked.esm.js` (and its embedded TypeScript sources), in safe `no_std`
Rust (`#![deny(unsafe_code)]`, zero `js!{}`, zero host `RegExp`). It matches `marked@18.1.1`
character for character on all 111 corpus cases in Node, Chrome, and Firefox (both minified and
readable artifacts). The 2026-11-20 pre-merge review found four classes of divergence in the
hand-written matchers (regex `.` line terminators, UTF-16 counts, and two backtracking cases); they
are fixed. Re-run on the fixed build, the review's differential fuzzers (B1's 194,000 and B2's
1.09 million inputs) and a targeted fuzzer for the fixed classes (400,000 inputs with `\r`, U+2028,
U+2029, astral text, lone pipes and pedantic links, across all 8 `(gfm, breaks, pedantic)`
combinations and both operations) find no divergence except the nesting-depth limit below.

| File | What it is |
| --- | --- |
| `src/lib.rs` | `#![no_std]` / `#![no_main]` / `#![deny(unsafe_code)]` entry, `#[panic_handler]` throwing via `mires_js::throw_value`, `matrix_run` export |
| `src/boundary.rs` | `JsValue` input decoding (`op`, `markdown`, `options: { gfm, breaks, pedantic }`) and raw HTML string return |
| `src/unicode.rs` | Immutable binary-search tables generated from `libraries/marked/unicode-classes.json` (`PUNCT`, `OPEN_QUOTE`, `ALNUM_BMP`, JS `\s`), UTF-8 decoders, and `PrevChar` (modeling UTF-16 `token.raw.slice(-1)` including `AstralLowSurrogate`) |
| `src/tokens.rs` | `Options`, `BlockToken`, `InlineToken`, `ListItem`, `TableCell`, `Align`, `LinkDef`, `InlineSlot` arena |
| `src/helpers.rs` | `preprocess`, `escape`, `unescape`, `clean_url`, `split_cells`, `rtrim_byte`, `find_closing_bracket`, `indent_code_compensation`, `normalize_label`, `is_html_entity` |
| `src/block_rules.rs` | Pure-Rust byte-slice scanners implementing every `marked@18.1.1` block rule across CommonMark, GFM, and pedantic modes (space, indented/fenced code, ATX & setext headings, hr, blockquote & lazy continuation, list items/tabs/indentation, 7-type HTML blocks, link defs, GFM tables, paragraphs) |
| `src/inline_rules.rs` | Pure-Rust byte-slice scanners implementing every `marked@18.1.1` inline rule (escape, tag, link/reflink/nolink, emStrong `*`/`_` flanking with punctuation/quote/alnum rules, codespan, br, del, autolink, GFM URL/email autolink with balanced-paren and punctuation trimming, inline text, `mask_inline_src`) |
| `src/lexer.rs` | `Lexer` state machine: `lex`, `lex_inline`, `block_tokens`, `inline_tokens`, `inline_tokens_inner`, `InlineSlot` queue draining, and all block/inline tokenizers matching `Tokenizer.ts` / `Lexer.ts` |
| `src/renderer.rs` | `render_blocks`, `render_inline`, and `render_text` (`Renderer.ts` + `TextRenderer.ts` + `Parser.ts`, including HTML-escaped `TextRenderer` output for image `alt` attributes) |

## Decisions

- **Hand-written rule matchers instead of host `RegExp`.** `no_std` Rust has no built-in regex
  engine, so each CommonMark, GFM, and pedantic regex from `src/rules.ts` is a hand-written
  byte-cursor matcher (`block_rules.rs`, `inline_rules.rs`) that decides what the regex would
  match. They are not a mechanical translation: a matcher scans forward and has to reproduce the
  regex's backtracking by reasoning about where it can succeed (for example the optional leading
  `|` of `lheadingGfm`'s table lookahead, `pedanticHrefTitle`'s `[^'"]*`, the pedantic `def`'s `<?`,
  and `reflinkSearch`'s `nolink(?!\()`, each commented at the matcher). Equivalence rests on the
  corpus and on differential testing against marked, not on construction. Unicode character
  classes (`\p{P}`/`\p{S}`, `[\p{Pi}\p{Ps}"']`, BMP alphanumeric, JS `\s`) use immutable sorted
  tables in `src/unicode.rs` generated from `libraries/marked/unicode-classes.json` so behavior is
  self-contained and identical across JS engines (`0` embedded JS, `0` host JS, `0` module state).
- **Regex `.` and `m`-flag lines.** No marked rule has the `s` flag, so a regex `.` matches any
  code unit except the line terminators `\n`, `\r`, U+2028 and U+2029. Matchers that stand for
  `.` (heading, lheading, `def`'s `<.*?>`, the GFM table header and rows, the link `<...>` escape,
  the pedantic link and `pedanticHrefTitle`) stop at all four through
  `unicode::line_terminator_len_at` / `dot_run_end`; matchers that stand for `[^\n]` stop at `\n`
  only (`block_rules::line_end`). The three `m`-flag rules (`codeRemoveIndent`,
  `blockquoteSetextReplace2`, `spaceLine`) iterate `block_rules::js_lines`, which splits at all
  four terminators. `parse` turns `\r` into `\n` first, but `parseInline` keeps it.
- **UTF-16 counts.** Positions are UTF-8 byte offsets, which is exact wherever JS uses a count as
  an offset into the same string. Where a count is compared with a bound or carried to another
  string it is converted to UTF-16 code units, as JS counts: the `_boundedBlockLabel` `{1,999}`
  bound (`match_block_label`: an astral character is two items), `Tokenizer.list`'s
  `line.slice(indent)` (`slice_utf16_from`) and the `linkLen` truncation (`utf16_len`). Only
  marked's `[...str]` spreads (`emStrong`/`del` delimiter lengths) and `expandTabs` count code
  points, and the port counts scalar values there.
- **Recursion depth (declared deviation).** Like marked, nested blocks recurse (`block_tokens` ->
  `tok_list` -> `block_tokens` for each list level). Each level holds two large mires-compiled
  frames, so under Node's default stack a list nested about 200-280 levels deep (`- - - ... a`)
  throws `RangeError`, where marked itself reaches about 2,490 levels; the host-`RegExp` ports
  reach about 600-1,000. Nesting that deep is not in the corpus; the flat token loops are iterative
  and run in constant stack (100 KB inputs of 50,000 tokens match marked).
- **Upstream behaviours kept.** Preserves subtle upstream behaviors rather than substituting
  generic CommonMark heuristics: `breaks` requires `gfm && !pedantic` (`pedantic` overrides grammar
  rules, while task lists still check `options.gfm`); `indentCodeCompensation` applies only to
  backtick fences, not tilde fences; paragraph HTML interrupts are case-sensitive while `block.html`
  and list HTML breaks are case-insensitive; `PrevChar::AstralLowSurrogate` matches JS
  `token.raw.slice(-1)` seeing a lone UTF-16 low surrogate after astral scalars; `tok_link` extra-`)`
  truncation computes `linkLen` in UTF-16 code units matching `Tokenizer.ts:783`; `mask_reflink`
  mirrors `match0.lastIndexOf('[')`; `clean_url` implements `encodeURI(s).replace(/%25/g, '%')`
  over UTF-8 bytes; and `merge_nested_list` handles multi-byte UTF-8 char boundaries safely while
  matching `marked`'s byte consumption.
- **Flat `Vec<InlineSlot>` arena for `inlineQueue`.** Models `Lexer.inlineQueue` (which in JS
  mutates `token.tokens` arrays in place after block lexing finishes and all `def` link references
  are collected) via `inline_id: usize` indices into `Lexer.inline_slots`, avoiding `Rc`/`RefCell`.
- **Byte-cursor `(src, bytes, pos)` lexing and lazy `masked_bytes`.** Block and inline lexers keep
  `bytes = src.as_bytes()` once and advance a `usize` cursor `pos` rather than repeatedly reslicing
  `&str` and calling `.as_bytes()` (see `PERF-003` below). `mask_inline_src` runs lazily within a
  single `inline_tokens_inner` call only if `*`, `_`, or `~` is encountered (zero cross-call state).
- **No `core::fmt` formatting on hot paths and no `String::from_utf8` on hot paths.** HTML output
  is assembled directly into a pre-sized `String` in `renderer.rs`, and decimal start numbers /
  heading depths are formatted with tiny ASCII byte helpers.

## mires bugs and performance cliffs

Three compiler/runtime bugs and one major `js-str=bytes` performance cliff were discovered while
porting `marked@18.1.1`, logged in `experiments/library-matrix/MIRES-BUGS.md` with standalone repros
under `experiments/library-matrix/mires-bugs/`:

- **`MIRES-017` (`mires-bugs/MIRES-017-switch-last-case-missing-break/`):** In `tok_blockquote`,
  the JIR structurizer emitted a multi-exit `switch (d)` whose final `case 1:` omitted `break;`
  before `default: throw new Error("rustc_codegen_js: bad block " + d)`.
  - *Workaround (`// WORKAROUND(MIRES-017)` in `src/lexer.rs`):* extracted `parse_blockquote_lines`
    as a non-`Option` helper using a `done` flag and separate `merge_nested_blockquote` /
    `merge_nested_list` helpers.
- **`MIRES-018` (`mires-bugs/MIRES-018-slice-is-ascii-unaligned-u32-view/`):** Under
  `js-str=bytes`, `str::is_ascii()` / `[u8]::is_ascii()` runs `core`'s SWAR unaligned `u32` loop and
  throws `byte offset N cannot form a u32 scalar view` on misaligned slices or lengths `>= 4` not
  divisible by 4.
  - *Workaround (`// WORKAROUND(MIRES-018)` in `src/helpers.rs`):* replaced `s.is_ascii()` in
    `normalize_label` with a simple `for &b in s.as_bytes()` check.
- **`MIRES-019` (`mires-bugs/MIRES-019-option-ref-const-decodes-none/`):** Promoted constant
  `Option<&u8>` values (`Some(&b'#')`, `Some(&b' ')`, `Some(&b'\t')`) silently decoded to
  `{ TAG: "None" }` in `tok_heading`, causing ATX closing `#` trimming to fail.
  - *Workaround (`// WORKAROUND(MIRES-019)` in `src/lexer.rs`):* compared `.last().copied()` against
    by-value `Some(b'#')` / `Some(b' ')` / `Some(b'\t')`.
- **`PERF-003` (`MIRES-BUGS.md`):** Initial port resliced `src = &src[consume..]` and called
  `src.as_bytes()` on each token step, thrashing `$rt_bstr_bytes`'s `memo.map` up to its 4 MB
  `memo.budget` (whereas `memo.huge` only triggers above 1 MB) under `js-str=bytes` (`71.3%` of V8
  CPU time in `$rt_bstr_bytes` + GC + `$rt__bstr_units`, ~66–70 ms in Node / ~25–28× JS).
  Refactoring block/inline lexers to hold `bytes = src.as_bytes()` once and scan with a `pos: usize`
  cursor brought Node down to ~15.1–16.1 ms (~6.6–6.9× JS) and Chrome to ~10.5–11.2 ms (~5.6–5.8× JS).
- **Remaining gap (`PERF-001`):** After eliminating the O(N²) `$rt_bstr_bytes` cliff, V8 CPU
  profiling shows the remaining ~6.6× gap in Node is dominated by `mires` heap/object traffic and
  short-string decoding (`16.2%` GC, `8.3% $rt__register`, `6.8% $rt_bstr_bytes` on short token
  strings, `6.8% $rt_dealloc`, `5.8% $rt__bstr_units`, `4.8% $rt__put`, `4.7% $rt__blk_get`).

## Size

After the 2026-11-20 review fixes `dist/index.min.js` is `289,307 B` (`85,738 B` gzip-9,
`68,688 B` brotli-11); the figures below are from the build before them.

`dist/index.min.js` was `288,482 B` (`85,290 B` gzip-9, `68,384 B` brotli-11; `7,129` lines;
`0` embedded JS, `0` eval refs, `0` host JS, `0` module state; `688` items: `438` JIR, `250`
non-ordinary fallback, `ordinaryFallback: 0`). By `optimizer-report.json` (`310,364 B` linked
`program.js` before Rollup/Terser):

- `matrix_marked`: `242,323 B` (377 items — largest functions: `tok_list` 14.2 KB,
  `inline_tokens_inner` 13.6 KB, `tok_html` 10.9 KB, `block_tokens` 10.8 KB, `tok_table` 4.5 KB,
  `match_tag` 4.2 KB, `match_def` 4.2 KB, `render_inline` 3.5 KB, `tok_link` 3.4 KB,
  `match_table_at` 3.4 KB; `unicode.rs` static range tables `ALNUM_BMP`/`PUNCT`/`OPEN_QUOTE`
  ~9.5 KB)
- `core`: `39,016 B` (43 items — dominated by Rust `core::unicode` case-conversion tables
  `UPPERCASE_LUT` 9.0 KB, `LOWERCASE_LUT` 6.5 KB, `case_ignorable` / `lowercase` / `uppercase`
  bitsets ~9.8 KB pulled in by `char::to_lowercase` for Unicode reference-link label folding)
- `generated` link wrappers: `14,903 B` (244 items)
- `alloc`: `13,528 B` (22 items)
- `mires_js`: `495 B` (2 items)
- Minimal `runtime/shim.js` subset in Rollup attribution: `36,749 B` (`36.7 KB`).
