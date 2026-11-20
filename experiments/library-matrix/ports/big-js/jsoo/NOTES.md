# big.js 7.0.1: js_of_ocaml port

A hand-written, faithful port of the subset in `libraries/big-js/CONTRACT.md` to OCaml, compiled
by js_of_ocaml 6.4.1 (whole program, `--opt=3`). It follows `node_modules/big.js/big.mjs` function
by function: `parse`, `round` (here `round_to`), `stringify`, `cmp`, `div`, `minus`, `plus`, `pow`,
`P.round` (`round`), `sqrt`, `times`, `toExponential`, `toFixed`, `toPrecision` and `toString`,
with upstream's loops, order of checks and error messages. It replaces a byte copy of a
generator-emitted Melange file.

| File | What it is |
| --- | --- |
| `src/big.ml` | the decimal record `t`, `Big_error`, upstream's functions, and `operate` (one reference operation) |
| `src/host.mli` | the three JS number conversions the core needs |
| `src/host.ml` | their js_of_ocaml implementation: `Number` through `Js.Unsafe.global`, and `toString`/`toExponential` methods of `Js.number` |
| `src/main.ml` | the boundary, doing what `libraries/big-js/reference.mjs` does; `Js.export "run"` |

**Shared source.** `big.ml` and `host.mli` are byte-identical to `ports/big-js/melange/src/`, as
the README allows; only `host.ml` and the entry module differ. The representation, numeric
semantics, square-root choice and deviations are therefore those in
`ports/big-js/melange/NOTES.md`. This file states only what differs.

## Boundary

- **Input.** The input and its cases are class types (`Js.readonly_prop`).
- **Options.** `dp`, `rm` and `repeat` are `Js.Opt.t Js.Optdef.t`, so `undefined` and `null` both
  take the default (20, 1, 1), as the adapter's `??` does.
- **Optional arguments.** `n` and `mode` are `Js.Optdef.t`, which become options and reach the core
  as upstream's optional arguments.
- **Operands.** Dispatched on `Js.typeof`:
  - a string crosses with `Js.to_string` (UTF-16 to UTF-8; a non-ASCII operand stays non-ASCII
    and is invalid, as in big.js);
  - a number −0 becomes `"-0"`;
  - anything else goes through JS `String()`.
- **Output.** The joined result returns through `Js.string`. All core strings are ASCII.

## Square root

As in Melange: upstream's iteration exactly, with no shortcut. `dp` ≤ −5 or a non-integer `dp`
raises `Invalid decimal places` through `div`. Where upstream's loop never stops, this port does
not stop either; those inputs are outside the domain (CONTRACT.md).

## Toolchain note

The build log prints `js_of_ocaml: Error: Unresolved internal primitive: %int_mul`. The cause is
the builder's second, runtime-attribution compile (`--no-runtime`), which js_of_ocaml 6.4.1
refuses for any `int` multiply. The builder records `runtime: null` and the production artifact is
unaffected. See `ports/minisearch/jsoo/NOTES.md` and `ports/fflate/jsoo/NOTES.md`.

## Performance accommodations

None.
