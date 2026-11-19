# Library contract: `floor`

The floor library measures each language's **runtime floor** the same way for every family: the
production artifact of the identity program. Its ports do nothing but the boundary (a JS string in,
the same string out), so what remains in `dist/index.min.js` is the language runtime the boundary
pulls in, the family's loading-only packaging (facade, module wrapper), and the minimal program.
The sizes table reports it beside every other port of the same language ("floor gzip-9").

This replaces the family-specific floors of the comparator builds (GopherJS's one-export empty
program, Elm's empty worker, js_of_ocaml's `--no-runtime` difference) with one definition. Each
builder still records its family-specific split of the emit in `dist/meta.json` (`attribution`).

| Field | Value |
| --- | --- |
| Package | `matrix-floor` 1.0.0, a stand-in: `package/index.mjs` |
| Reference adapter | `reference.mjs`: `run(input) = identity(input.text)` |
| Input | `{ text: string }`, well-formed UTF-16 |
| Output form | `string`, synchronous |
| Normalisation | none |
| Parity environments | node |
| Corpus | `corpus.json`: empty, ASCII, and BMP plus astral text with a ZWJ sequence |
| Bench | none (not timed) |
| Caching | none |

Ports may not avoid the boundary: a language whose strings are not JS strings (Rust, OCaml through
js_of_ocaml, Go) converts `text` into its own string type and back, exactly as a library port does.
