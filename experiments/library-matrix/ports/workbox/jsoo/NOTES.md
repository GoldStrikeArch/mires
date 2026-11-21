# workbox 7.4.1: js_of_ocaml port

This port is written by hand. It replaces the earlier port of the flattened scenario contract, which a Python translator (`build/library-matrix/dev-wb-comparators/`) generated from a shared core. It is a faithful port of workbox-precaching 7.4.1 and the workbox-core, workbox-routing and workbox-strategies parts it uses (the production bundles behind `matrix-entry.mjs`, the subset in `libraries/workbox/CONTRACT.md`), and of what `libraries/workbox/reference.mjs` does with them, to OCaml 5.3 compiled by js_of_ocaml 6.4.1.

**Shared source.** `src/host.mli`, `src/workbox_core.ml`, `src/strategies.ml`, `src/routing.ml`, `src/precaching.ml` and `src/adapter.ml` are byte-identical in the Melange and js_of_ocaml ports (README "Per-language notes"). Only `src/host.ml`, the per-compiler interop module, and the entry module differ. `ports/workbox/melange/NOTES.md` describes the shared files, their structure, idiom and deviations from the npm source, which hold for this port as written there.

| File | What it is |
| --- | --- |
| `src/workbox_core.ml`, `src/strategies.ml`, `src/routing.ml`, `src/precaching.ml`, `src/adapter.ml`, `src/host.mli` | shared with the Melange port |
| `src/host.ml` | the host interface for js_of_ocaml: its `Promise` binding, `Js.regExp`, and `Js.Unsafe` for CacheStorage, Fetch, URL and the script's and the output's plain values |
| `src/main.ml` | the entry module: publishes `run` with `Js.export` |

## Boundary

`run(input)` reads the script in place, member by member, through `Host` (`Js.Unsafe.any` values). Manifest entries, revisions, response bodies and the fail fixture's `value` stay JS values. Workbox's logic runs over `Host`'s abstract types for the host's CacheStorage, Cache, Request, Response, Headers and URL. The output is a JS object built through `Host`, in the contract's key order, and `run`, published with `Js.export`, returns a promise of it.

## Interop decisions for this compiler

- **Promises** are js_of_ocaml's `Promise.t`, so each outcome is a host promise whose continuations are microtasks. `Promise.of_any` and `Promise.to_any` cross the host's own promises. The outcomes resolve with OCaml results, which are never thenables, so the binding never wraps them.
- **Strings.** OCaml strings are UTF-8 here, so a string crosses with `Js.string` and `Js.to_string`. The strings that cross are URLs (always ASCII once serialised), cache names, method names, search parameter names and messages, all well-formed, so the conversion is exact. The shared code compares only whole strings and ASCII prefixes, so UTF-8 and UTF-16 give the same decisions. `Host.replace_all` is JS's `replaceAll`, and the ignore patterns are JS `RegExp`s tested on the JS string.
- **Exceptions.** A synchronous JS exception (a URL or Request constructor's TypeError, a RegExp's SyntaxError) is caught as `Js_error.Exn` and returned as the JS error it holds. js_of_ocaml makes that `Js_error.Exn` only of a value that is `instanceof Error` in its realm, which every host error is here: the checker runs the port in the page's own realm (CONTRACT.md "Realm"). A rejection keeps its value as it is (`Promise.error_to_any`).
- **Host objects** are `Js.Unsafe.any` behind the interface's abstract types. `caches` and `location` are read at every call, as Workbox reads them.

## Deviations from the npm source

The shared source's, in `ports/workbox/melange/NOTES.md`. None changes a result in the parity domain.

## Module state

The shared source's plugin constants, as in the Melange port. Nothing else outlives a call.

## Performance accommodations

None.
