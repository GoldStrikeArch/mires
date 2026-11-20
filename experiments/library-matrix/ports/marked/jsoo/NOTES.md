# marked 18.1.1: js_of_ocaml port

A faithful port of the synchronous CommonMark and GFM compilation pipeline in
`libraries/marked/CONTRACT.md`, written as **one OCaml program compiled by two compilers**: this
directory (js_of_ocaml 6.4.1) and `../melange/` (Melange 7.0.1) share the same 9-module OCaml core
byte-for-byte, differing only in `src/host.ml` and the entry module (`src/main.ml` vs
`src/index.ml`). It reaches character-for-character parity with `marked@18.1.1` on all 80 corpus
cases in Node, Chrome, and Firefox (both minified and readable artifacts), plus 1,638 adversarial
and randomized differential cases across all 8 `(gfm, breaks, pedantic)` combinations and both
`parse` and `parseInline`.

| File | Shared with `../melange/` | What it is |
| --- | --- | --- |
| `src/host.mli` | identical | JS host interface: `Vec` (JS `Array`), `Jstr` (native JS UTF-16 string), `Re` (host `RegExp` with zero-allocation nullable match inspection and pre-wrapped stateless replacers), `StrMap` (JS `Map`), `Value` (in-place JSON input reader) |
| `src/rules.ml` | identical | Exact port of `marked@18.1.1` `src/rules.ts`: `Other`, `edit` builder, precomputed 4-entry indent regex tables (`0..3`), `block.{normal,gfm,pedantic}`, and `inline.{normal,gfm,breaks,pedantic}` |
| `src/helpers.ml` | identical | Exact port of `src/helpers.ts`: `escape_html_entities`, `decode_numeric_character_references`, `clean_url`, `split_cells`, `rtrim`, `trim_trailing_blank_lines`, `normalize_label`, `find_closing_bracket`, `expand_tabs`, plus UTF-16 code-point helpers |
| `src/tokens.ml` | identical | `options`, `link_def`, `align`, block/inline `token` variants, `inline_queue_entry`, `lexer_state`, `lexer` |
| `src/tokenizer.ml` | identical | Exact port of `src/Tokenizer.ts` (all block and inline tokenizers) |
| `src/lexer.ml` | identical | Exact port of `src/Lexer.ts` (block lexer, FIFO `inline_queue`, 3-pass delimiter masking, inline lexer) |
| `src/renderer.ml` | identical | Exact port of `src/Renderer.ts` and `src/TextRenderer.ts` |
| `src/parser.ml` | identical | Exact port of `src/Parser.ts` (block `parse`, inline `parse_inline`, image-alt `parse_inline_text`) |
| `src/script.ml` | identical | `run(input)`: decodes `{ op, markdown, options? }` and dispatches `parse` / `parseInline` |
| `src/host.ml` | jsoo only | `Js_of_ocaml` host over `Js.js_string`, `Js.regExp`, `Js.js_array`, JS `Map`, `Js.Unsafe.callback_with_arity` |
| `src/main.ml` | jsoo only | Classic-script export: `Js.export "run" (Js.wrap_callback Script.run)` |

`diff -rq src/ ../melange/src/` confirms only `host.ml` differs and `main.ml` vs `index.ml` are
the per-compiler entry points. Shared core design notes are in `../melange/NOTES.md`; what follows
is specific to js_of_ocaml.

## Interop decisions

- **Native `Js.js_string` end-to-end.** `Host.Jstr.t` is `Js.js_string Js.t` (never converted via
  `Js.to_string` except for the short `"parse"` / `"parseInline"` op name in `script.ml`). Extra
  ES2015+/ES2019 methods (`trimStart`, `trimEnd`, `includes`, `endsWith`, `repeat`, integer-typed
  `charCodeAt`) are bound via a zero-overhead `class type ext_js_string` inheriting `Js.js_string`.
- **Direct `Js.regExp` and strict-arity replacers.** `String.prototype.replace(re, fn)` passes
  `(match, ...groups, offset, wholeStr, groupsObj)` — more arguments than a 1-arg or 3-arg OCaml
  callback accepts. Using `Js.Unsafe.callback_with_arity 1` and `Js.Unsafe.callback_with_arity 3`
  (compiled to `caml_js_wrap_callback_strict`, which truncates `args.length = arity`) prevents
  `Js.wrap_callback` over-application crashes. Wrapping the 5 pure stateless replacers once at
  module load via `Re.make_replacer1` / `Re.make_replacer3` eliminates per-call wrapper allocation.
- **Zero-copy `Js.js_array` (`Vec`) and zero-argument `Map` (`StrMap`).** `split` and `join`
  operate directly on `Js.js_array` without converting to/from OCaml tagged arrays. `Map` is typed
  as a zero-argument `Js.constr` (`new%js map_constr`, no `()` argument, avoiding `new Map(0)`).
- **Zero cross-call mutable state.** `0` embedded JS, `0` eval refs, `0` host JS, `0` module-state
  lint findings.

## js_of_ocaml 6.4.1 compiler findings

1. **Over-application in `String.prototype.replace` callbacks.** `Js.wrap_callback` invokes
   `caml_callback` with all JS arguments and tries to call the returned string as a function when
   extra arguments (`offset`, `input`, `groups`) remain. `Js.Unsafe.callback_with_arity` cleanly
   truncates `arguments` to the declared arity.
2. **Whole-program inlining and `--no-runtime` attribution.** Under `--opt=3`, whole-program
   inlining collapses the `Host` module wrappers directly into `Tokenizer`, `Lexer`, and `Renderer`.
   Because `marked` only uses power-of-two/constant `mod` (and no general integer multiply/divide),
   `js_of_ocaml compile --no-runtime` succeeds cleanly here (`program: 42,122 B`, `runtime:
   65,422 B` before facade/Terser).
3. No js_of_ocaml miscompilation was encountered; clean rebuilds are byte-identical.

## Size and quick Node timing observations

In isolated dev verification (`build/library-matrix/dev-marked-ocaml/`):
- `dist/index.min.js`: **102,195 B** raw, **34,125 B** gzip-9, **30,048 B** brotli-11 (emit before
  facade/Terser: 107,544 B = 42,122 B program + 65,422 B runtime; JS reference is 46,922 B raw /
  13,794 B gzip-9 / 12,495 B brotli-11).
- Quick Node check (6 samples, triplet design, load ~4.5): `bench-parse` 1.28× JS (3.02 ms vs
  2.36 ms, A/A 1.040), `bench-gfm` 1.33× JS (3.09 ms vs 2.32 ms, A/A 1.014). Official matrix
  receipts will be recorded under `build/library-matrix/`.
