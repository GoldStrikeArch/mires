# marked 18.1.1: Melange port

A faithful port of the synchronous CommonMark and GFM compilation pipeline in
`libraries/marked/CONTRACT.md`, written as **one OCaml program compiled by two compilers**: this
directory (Melange 7.0.1) and `../jsoo/` (js_of_ocaml 6.4.1) share the same 9-module OCaml core
byte-for-byte, differing only in `src/host.ml` and the entry module (`src/index.ml` vs
`src/main.ml`). It matches `marked@18.1.1` character for character on all 111 corpus cases in Node,
Chrome, and Firefox (both minified and readable artifacts). The 2026-11-20 pre-merge review found
two defects in the shared core, both fixed: a stray `}` in `htmlBeginRegex` (`ul}`) and lexer loops
that recursed once per token (see "Constant-stack lexer loops" below). Every
rule regex the core builds now has the source and flags of the matching compiled marked rule, up
to `\t` spelled for a literal tab (all `RegExp` constructions captured through a proxy and compared
with `Lexer.rules` and `rules.other`), and the review's differential fuzzers find no divergence.

| File | Shared with `../jsoo/` | What it is |
| --- | --- | --- |
| `src/host.mli` | identical | JS host interface: `Vec` (JS `Array`), `Jstr` (native JS UTF-16 string), `Re` (host `RegExp` with zero-allocation nullable match inspection and pre-wrapped stateless replacers), `StrMap` (JS `Map`), `Value` (in-place JSON input reader) |
| `src/rules.ml` | identical | Exact port of `marked@18.1.1` `src/rules.ts`: `Other`, `edit` builder, precomputed 4-entry indent regex tables (`0..3`), `block.{normal,gfm,pedantic}`, and `inline.{normal,gfm,breaks,pedantic}` |
| `src/helpers.ml` | identical | Exact port of `src/helpers.ts`: `escape_html_entities`, `decode_numeric_character_references`, `clean_url`, `split_cells`, `rtrim`, `trim_trailing_blank_lines`, `normalize_label`, `find_closing_bracket`, `expand_tabs`, plus UTF-16 code-point helpers (`code_point_count`, `first_code_point_len`, `first_line`) |
| `src/tokens.ml` | identical | `options`, `link_def`, `align`, block/inline `token` variants (sharing `text_token` with `mutable t_is_paragraph` for tight-to-loose list promotion), `inline_queue_entry`, `lexer_state`, `lexer` |
| `src/tokenizer.ml` | identical | Exact port of `src/Tokenizer.ts`: all block tokenizers (`space`, `code`, `fences` + `indent_code_compensation`, `heading`, `hr`, `list` + GFM task checkboxes, `blockquote` + nested/list continuations, `html`, `def`, `table`, `lheading`, `paragraph`, `block_text`) and inline tokenizers (`escape`, `tag`, `link` + `is_label_end_inside_token` + `output_link`, `reflink`, `em_strong`, `codespan`, `br`, `del`, `autolink`, `url` + `_backpedal`, `inline_text`) |
| `src/lexer.ml` | identical | Exact port of `src/Lexer.ts`: CRLF/CR normalisation, `block_tokens`, FIFO `inline_queue` draining, 3-pass `masked_src` construction (`reflink_search` + `link_in_text`, `any_punctuation_outside_code`, `block_skip`), `inline_tokens` with `link_paren_possible` guard |
| `src/renderer.ml` | identical | Exact port of `src/Renderer.ts` and `src/TextRenderer.ts` |
| `src/parser.ml` | identical | Exact port of `src/Parser.ts`: block `parse`, inline `parse_inline`, and image-alt `parse_inline_text` |
| `src/script.ml` | identical | `run(input)`: decodes `{ op, markdown, options? }` and dispatches `parse` / `parseInline` |
| `src/host.ml` | Melange only | Melange externals (`[@@mel.new]`, `[@@mel.send]`, `[@@mel.get]`, `[@@mel.set]`, `[@@mel.get_index]`, `[@@mel.set_index]`, `[@@mel.scope]`), uncurried `[@u]` regex replacers, `Js.Json` |
| `src/index.ml` | Melange only | ES module entry: `let run input = Script.run input` |

`diff -rq src/ ../jsoo/src/` confirms only `host.ml` differs and `index.ml` vs `main.ml` are the
per-compiler entry points.

## Interop and algorithm decisions

- **Native UTF-16 strings and host `RegExp` end-to-end.** `Host.Jstr.t` is `string` (native JS
  UTF-16), so no UTF-8 conversion occurs at the boundary or internally and every UTF-16 offset
  (extra-`)` link truncation, `prevChar` single-code-unit slicing, `isLabelEndInsideToken`) matches
  `marked@18.1.1` identically. All grammar rules compile to host JS `RegExp` once at module load,
  preserving exact `u`-flag Unicode property escapes, negative lookbehind, and named backreferences.
- **Strict ASCII literals in shared OCaml source.** Because js_of_ocaml's `Js.string` interprets
  OCaml string literals as Latin-1 while Melange emits them as UTF-16, all literals passed to
  `Jstr.of_string` / `Re.of_string` are pure 7-bit ASCII; `U+FFFD` in
  `decode_numeric_character_references` is built via `Jstr.from_code_point 0xFFFD`.
- **Zero-allocation regex match inspection and pre-wrapped stateless replacers.** Because
  `Host.Jstr.t` and `Host.Re.match_result` are abstract across `host.mli`, returning OCaml `option`
  from `Re.exec` / `Re.match_group` caused Melange to allocate `Caml_option.some` / `valFromOption`
  wrappers on every regex match. Switching `Host.Re` to nullable `exec_raw` + `is_match` +
  `match_group_or_empty` + `match_group_is_nonempty`, hoisting stateless regex replacers
  (`escape_replacer`, `decode_ncr_replacer`, `expand_tabs_zero_replacer`,
  `mask_punct_outside_code_replacer`, `mask_block_skip_replacer`) via `Re.make_replacer1` /
  `make_replacer3`, and unboxed loops in `Lexer` cut Node `bench-parse` from 1.73× JS down to
  ~1.32× JS and `bench-gfm` from 1.60× JS down to ~1.32× JS.
- **Constant-stack lexer loops.** marked's `blockTokens` and `inlineTokens` are `while` loops. The
  port's `block_tokens` / `inline_tokens_inner` loops are local `let rec loop`s whose every
  recursive call is a direct self tail call (`loop (push_token tokens s tok) ...`), which Melange
  and js_of_ocaml both compile to a JS `while` loop. Until the 2026-11-20 review, most branches
  went through a local `advance` closure that called `loop`; neither compiler turns that into a
  loop, so the stack grew by one frame per token and inputs of about 2,500-3,000 tokens (12 KB of
  `# h\n`) threw `RangeError`. Both builds now lex 100 KB inputs of up to 50,000 tokens (headings,
  emphasis, links, lists, tables, URLs, code spans, escapes; `parse` and `parseInline`) exactly as
  marked does, and the corpus pins 5,000 headings and 5,000 emphasis runs.
- **Two marked rules are not built.** `inline.del` is defined in `rules.ts` but never read by
  marked's `Tokenizer` (strikethrough goes through `delLDelim` / `delRDelim`), so `rules.ml` omits
  it; `other.findPipe` (`/\|/g`, used by `splitCells` to put a space before every unescaped `|`)
  is replaced by an `indexOf` loop in `helpers.ml`'s `split_cells` with the same result.
- **Native JS `Array` (`Vec`) and `Map` (`StrMap`).** Token vectors, `inline_queue`, and `split`/`join`
  buffers wrap JS `Array` directly with zero copying; `tokens.links` wraps JS `Map` (avoiding both
  prototype-key collisions like `__proto__`/`constructor` and Melange's `Hashtbl`/`Printf` pull-in).
- **Zero cross-call mutable state.** Only immutable compiled regex tables (including eager 4-entry
  indent tables replacing `marked`'s lazy `cachedIndentRegex` arrays) and stateless replacer
  functions exist at module scope (`0` embedded JS, `0` eval refs, `0` host JS, `0` module-state
  lint findings).

## Melange 7.0.1 compiler findings

1. **Abstract host types box into `Caml_option.some`.** Across an `.mli` boundary, Melange cannot
   prove an abstract type never contains `undefined`, so returning `match_result option` or
   `Jstr.t option` emits `Caml_option.nullable_to_opt` / `undefined_to_opt` + `valFromOption`
   on every match. Exposing `is_match` and `match_group_or_empty` on the abstract nullable handle
   eliminated that allocation from the hot lexer loops.
2. **Stdlib discipline.** Avoiding `Hashtbl`, polymorphic `=`/`<>`, `Array.init`, recursive value
   records, and `Stdlib.max_int` keeps the Rollup stdlib runtime to 7,366 B before Terser
   (`caml_obj.js` drops to 364 B for `caml_obj_dup` on record-with updates of the 21-field
   `inline_rules` record).
3. No Melange miscompilation was encountered; clean rebuilds are byte-identical.

## Size and quick Node timing observations

In isolated dev verification (`build/library-matrix/dev-marked-ocaml/`):
- `dist/index.min.js`: **57,483 B** raw, **16,330 B** gzip-9, **14,466 B** brotli-11 (Rollup
  attribution before Terser: 110,570 B program + 7,366 B Melange stdlib runtime; JS reference is
  46,922 B raw / 13,794 B gzip-9 / 12,495 B brotli-11).
- Quick Node check (6 samples, triplet design, load ~4.5): `bench-parse` 1.32× JS (3.15 ms vs
  2.39 ms, A/A 1.025), `bench-gfm` 1.32× JS (3.10 ms vs 2.36 ms, A/A 0.992). Official matrix
  receipts will be recorded under `build/library-matrix/`.
