# Ky 2.2.0: js_of_ocaml 6.4.1 port

This port is written by hand. It replaces the earlier port that `build/library-matrix/dev-ky/translate.py` generated from a shared core. It is a faithful port of Ky 2.2.0 (`distribution/index.js`, the subset in `libraries/ky/CONTRACT.md`) and of what `libraries/ky/reference.mjs` does with it, to OCaml 5.3 compiled by js_of_ocaml 6.4.1. It follows Ky's own functions: `normalizeRetryOptions`, `validateTimeoutOption`, the constructor, `#run`, `#retry`, `#retryFromError`, `#fetch`, `#calculateRetryDelay`, `#calculateDelay`, `#getResponseData`, `#throwProcessedError`, `utils/timeout.js` and `core/retry-timing.js`.

**Shared source.** `src/host.mli`, `src/ky.ml`, `src/retry_timing.ml` and `src/adapter.ml` are byte-identical in the Melange and js_of_ocaml ports (README "Per-language notes"). Only `src/host.ml`, the per-compiler interop module, and the entry module differ. `ports/ky/melange/NOTES.md` describes the shared files, their interop decisions and the deviations from the npm source, which hold for this port as written there.

| File | What it is |
| --- | --- |
| `src/ky.ml`, `src/retry_timing.ml`, `src/adapter.ml`, `src/host.mli` | shared with the Melange port |
| `src/host.ml` | the host interface for js_of_ocaml: its `Fetch`, `Abort` and `Promise` bindings, and `Js.Unsafe` for what they leave out |
| `src/main.ml` | the entry module: publishes `run` with `Js.export` |

## Boundary

Each corpus case is read in place, field by field. The option values Ky checks stay `Host.value`s (`Js.Unsafe.any`, `option` where undefined) until Ky's own checks read them with `typeof`, `Array.isArray` and `Number.isInteger`. Each result is the JS array `[trace, outcome]`, and `run` returns a promise.

## Interop decisions for this compiler

- **Bindings.** Headers, Request and Response are js_of_ocaml's `Fetch` class types, AbortController and AbortSignal its `Abort` ones, and the promises its `Promise.t`. `Js.Unsafe` reaches `AbortSignal.any`, `TextDecoder`, `setTimeout`, `Number`, `Reflect.has`, `Object.prototype.toString` and a Response built from the corpus's raw status and headers. `Js.regExp_withFlags` runs Ky's patterns.
- **Strings.** OCaml strings are UTF-8 here, so a string crosses with `Js.string` and `Js.to_string`. The strings Ky reads (header values, URLs, bodies, error data) are well-formed, so the conversion is exact. `Host.lowercase`, `uppercase` and `trim` convert to a JS string and call JS's own methods. The shared code compares only ASCII pieces byte by byte (`;`, the separator before `json`, the HTTP-date captures), so UTF-8 and UTF-16 positions agree on every decision.
- **Exceptions.** A synchronous JS exception is caught as `Js_error.Exn` and returned as the JS error it holds (`Host.catching`). js_of_ocaml raises `Js_error.Exn` for a thrown value that is `instanceof Error`, which every error a host constructor throws is.

## Deviations from the npm source

The shared source's, in `ports/ky/melange/NOTES.md`. None changes a result in the parity domain.

## Performance accommodations

The shared source's (`lowercase_headers` O(N) `Hashtbl` merge), described in `ports/ky/melange/NOTES.md`.
