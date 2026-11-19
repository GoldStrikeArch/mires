# MiniSearch 7.2.0: js_of_ocaml port

A faithful port of the subset in `libraries/minisearch/CONTRACT.md`, written as **one OCaml program
compiled by two compilers**: this directory (js_of_ocaml 6.4.1) and `../melange/` (Melange 7.0.1)
hold the same core, byte for byte, and differ only in the host module and the entry. It reaches
exact parity (doubles included) on all 58 corpus cases in Node, Chrome and Firefox, in the
harness's official receipts.

| File | Shared with `../melange/` | What it is |
| --- | --- | --- |
| `src/host.mli` | identical | the interface to JavaScript the core sees: `Text` (JS strings), `Value` (reading the input), `Json` (building the result) |
| `src/collections.ml` | identical | `Vec` (growable array) and `Ordered` (an insertion-ordered map: a JS `Map`'s semantics over `Map.Make`) |
| `src/searchable_map.ml` | identical | `SearchableMap`: the radix tree, `createPath`, `lookup`, `trackDown`/`remove`/`cleanup`/`merge`, `atPrefix`, the `TreeIterator` order, the Levenshtein walk of `fuzzySearch` |
| `src/options.ml` | identical | option types, the library defaults and their spread-merge (`{ ...a, ...b }`) |
| `src/minisearch.ml` | identical | the `MiniSearch` class method by method: add/remove/discard/replace, field lengths, vacuuming, search (query trees, `executeQuerySpec`, `termResults`, BM25+, the combinators, result assembly), autoSuggest, `toJSON` |
| `src/script.ml` | identical | `run(input)`: decodes the operation script, runs it, builds the result records |
| `src/host.ml` | jsoo only | `Host` with js_of_ocaml's typed method calls, `Js.typeof`, `Js.Unsafe.get`/`obj`, `Js.array` |
| `src/main.ml` | jsoo only | the entry: `Js.export "run" (Js.wrap_callback Script.run)` |

`shasum -a 256 src/{host.mli,collections.ml,searchable_map.ml,options.ml,minisearch.ml,script.ml}`
prints the same six digests here and in `../melange/`. The design of the core (data structures,
walks with deletions, the fuzzy matrix, vacuuming) is described in `../melange/NOTES.md`,
"Interop decisions"; what follows is what is specific to js_of_ocaml.

## Interop decisions

- **Text never becomes an OCaml string.** OCaml strings are bytes and `Js.to_string` converts
  UTF-16 to UTF-8, which loses lone surrogates and changes every length, slice and edit distance
  (CONTRACT.md, "Strings are UTF-16"). `Host.Text.t` is `Js.js_string Js.t`: document text, terms,
  radix labels, string ids, queries and messages stay JavaScript strings from the input to the
  result. The primitives are typed method calls (`##.length`, `##slice`, `##concat`,
  `##toLowerCase`, `##split_regExp` with `new%js Js.regExp_withFlags ... "u"`, `Js.strict_equals`,
  `(Js.number_of_float x)##toString`); js_of_ocaml's whole-program compile inlines them into the core
  as plain `s.charCodeAt(i)`, `s.slice(a, b)`, `a === b`.
- **`charCodeAt` returns an `int`** through a one-method class type (`code_units`): the binding in
  `Js.js_string` returns a `Js.number Js.t`, and a code unit is an integer, which is what a
  js_of_ocaml `int` is at run time.
- **Names are OCaml strings** (`Host.Value.name`, `Js.to_string`): field names, stored field names,
  the id field, option and operation names. The parity domain makes them ASCII identifiers, so the
  UTF-8 conversion is exact for them; they go back with `Js.string` as object keys and in messages.
- **The boundary**: the input is a `Js.Unsafe.top Js.t`, classified with `Js.typeof`,
  `Array.isArray` (through a typed class on `Js.Unsafe.global##._Array`) and `== null`, read with
  `Js.Unsafe.get` (`undefined` is `None`), never written; arrays are copied into OCaml arrays with
  `Js.to_array`. The result is built with `Js.Unsafe.obj` (keys in the order of CONTRACT.md
  "Output"), `Js.array`, `Js.bool` (an OCaml `bool` is 0 or 1 at run time) and the JS strings
  themselves. `Math.log` is `Float.log`, which js_of_ocaml emits as `Math.log`.
- js_of_ocaml's `int` is 32-bit; frequencies, short ids and counts fit. Numeric ids are doubles
  (`Number_id of float`), so any JSON number id is kept exactly.

## Toolchain problem: the runtime-attribution compile fails (resolved in the builder)

`languages/jsoo/build.mjs` compiles the production artifact (that step succeeds), then, for
`meta.json`'s runtime attribution, compiles the same bytecode again with `js_of_ocaml compile
--no-runtime`. js_of_ocaml 6.4.1 refuses that on any program whose code keeps an `int` multiply,
divide or modulo:

```
js_of_ocaml: Error: Unresolved internal primitive: %int_mul
```

and the builder fails the whole build. Minimal repro (a two-line program, `a * b`):
`build/library-matrix/jsoo-bugs/no-runtime-int-arith/` (`bash run.sh`, README.md there). `a * 31`,
`2 * a`, `a / 2` (`%int_div`), `a mod b` (`%int_mod`) and even `(a lsl 5) - a` (folded into a
multiply) fail the same way; the pilot and floor ports happen to contain none. The core does
ordinary `int` arithmetic of this kind in four places (the fuzzy matrix's row offsets, the growth of
two growable arrays, a binary search's midpoint). It was **not** rewritten around the bug:
rewriting OCaml arithmetic to dodge a measurement step of the harness would distort the program
being measured, and the optimiser's folding makes such a rewrite unpredictable. Resolved on 2026-11-19 in the harness: `languages/jsoo/build.mjs` now catches a failure of the
diagnostic attribution compile and records `runtime: null` plus `runtimeUnavailable`. The production
compile, flags, facade and Terser pass are unchanged, and floor/jsoo and pilot/jsoo rebuild
byte-identically. All results for this port are in the official receipts under `build/library-matrix/`.
(The earlier provisional sandbox, `build/library-matrix-minisearch-jsoo-provisional/`, produced the same
artifact and is kept only as history.)

## Language and compiler findings (js_of_ocaml)

1. The `--no-runtime` defect above.
2. `Hashtbl` would drag `Random`/`Domain`/`Printexc`/`Printf` into the Melange bundle (65.6 KB
   minified for one table; `../melange/NOTES.md`), so the shared core uses `Map.Make`. Under
   js_of_ocaml the switch from `Hashtbl` cut the artifact from 105,895 to 97,010 bytes; the
   identity program (`floor`) is already 67 KB here, most of it runtime.
3. Unknown-function calls go through `caml_call1`/`caml_call2` (an arity check), as Melange's
   curried calls do; the hot loops of the core are direct recursion either way.

No js_of_ocaml miscompilation was found.

## Less idiomatic than plain js_of_ocaml code

Written alone for js_of_ocaml, this port would use `Js.js_string Js.t` (or an `int array` of code
units) directly instead of the abstract `Host` interface, which exists so that the same core also
compiles with Melange.

## Size and time

`dist/index.min.js` is 96,987 bytes (31,920 gzip-9, 27,993 brotli-11), against the js_of_ocaml floor
of 67,208 (21,349 gzip-9). The runtime attribution in `meta.json` is `null` (`runtimeUnavailable`
gives the reason), because the attribution compile is the step that fails. Current figures and
timings are in `build/library-matrix/sizes.json`, `bench/minisearch/` and `MATRIX.md`.

The js_of_ocaml runtime installs a global error handler at import (`process.on("uncaughtException")`
in Node, a window `error` listener in browsers). The floor and pilot ports do the same, so this belongs
to the family, not to this port.
