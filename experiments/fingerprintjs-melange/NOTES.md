# Melange 7.0.1 notes

FingerprintJS 5.2.0, compiled by Melange `7.0.1-53` on OCaml 5.3.0
(`fingerprintjs-compare` switch). The emit is import-free: no
`melange.js/*` and no `melange/stdlib.js`. Packed ESM therefore has no
runtime to count.

## IMPORT-FREE BY DESIGN

Melange's runtime lives in `melange.js` and is emitted as ordinary ES
imports (`melange.js/caml_array.js` and friends). An output that imports
one cannot be weighed honestly: its real size would be its own bytes
plus a runtime the comparison has to pack. Every idiom below was chosen
so the emit has no import at all, each verified against this 7.0.1-53
switch rather than assumed.

  * `a.(i)` and everything else in `Stdlib.Array`  ->  `melange.js/caml_array.js`
    (the bounds check is a runtime call). Indexing is `[@@mel.get_index]`
    / `Js.Array.unsafe_get`, which compile to `a[i]`.
  * `Js.Math.*`  ->  `melange.js/js_math.js`. In v7 `Js.Math` is a module,
    not a wall of externals. Used Math functions are declared locally with
    `[@@mel.scope "Math"]`.
  * integer `mod` by a NON-constant divisor  ->  `melange.js/caml_int32.js`.
    Remainders that are not constant-folded stay in JS (`%` / `>>>`).
  * `Js.Exn`, `raise`, and `try/with` on OCaml exceptions  ->
    `melange.js/caml_exceptions.js`, `caml_js_exceptions.js`, `js_exn.js`.
    Host failures go through a local `try_host` (`try/catch` in
    `[%mel.raw]`).
  * `Js.Dict.get`  ->  `melange.js/caml_option.js` (the `Some` conversion).
    Property reads are `[@@mel.get_index]` and keep `undefined` as a JS
    value.
  * `string_of_int` / `string_of_float` / `Format`  ->  `melange.js/caml_format.js`.
    Numbers become strings with `.toString` / `String(...)`.
  * OCaml `infinity`  ->  `melange/stdlib.js`. Architecture uses
    `[%mel.raw {| Infinity |}]`.
  * A `[%mel.raw {| function() { ... } |}]` bound at `unit -> _`  ->
    `melange.js/curry.js`. JS length 0 is arity-encoded. Zero-arg helpers
    are OCaml `let f () = [%mel.raw {| expr |}]` (emitted length 1) or
    take an explicit dummy.
  * Passing a multi-arg OCaml function into JS and calling it as
    `f(a)(b)` or `f(a, b)` without checking `.length`  ->  either
    `curry.js` or a wrong call (uncurried length-2 `f(a)` already runs
    the body). Wrappers inspect `fn.length` and pick one call or the
    curried chain.
  * Polymorphic `=` on unknown JS values  ->  `caml_obj` / `caml_equal`.
    Strings and numbers use `=` / `==`; host objects use `===`.

What survived unchanged is most of the language: `^`, `ref`, `for` /
`while`/`rec`, `land`/`lor`/`lsl`/`lsr` on values that stay below the
sign bit, array literals used only as compile-time tables.

## HASHING

OCaml `int` in this Melange is signed 32-bit. The murmur x64 lanes set
bit 31; doing the 16-bit schoolbook arithmetic in OCaml corrupted
`h1[0]` (`hello` hashed to `3427584d…` instead of `cbd8a7b3…`). The
inner loop is therefore the TypeScript body kept on JS numbers via
`[%mel.raw]`. UTF-8 is the oracle's `charCodeAt` / `TextEncoder` path,
including unpaired surrogates → U+FFFD.

## OTHER RAW BODIES

`getFilters` is the oracle table (including `atob` selectors) as one
`[%mel.raw]` IIFE so the 40-odd filter lists cannot drift. Everything
that decides which probe to run, in which order, with which timeout,
is OCaml.

## PUBLIC SURFACE

Named exports, default-export identity, the 42-key `sources` object
(with oracle `.name` / `.length`), `mapWithBreaks` (16 ms `Date.now()`
budget, `MessageChannel` yield, non-awaited callback starts, final
`Promise.all`), and `LoadOptions.monitoring` as a no-op all match
`CONTRACT.md`.
