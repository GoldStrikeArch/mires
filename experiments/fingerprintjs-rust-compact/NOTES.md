# Compact Rust deletion log

Copy of the frozen idiomatic port, then mutated in place. Algorithm version stays `5.2.0`. Package/crate identity is `fingerprintjs-rust-compact-experiment`. No `js!`, no product `.js`, `unsafe_code = "deny"`, `no_std`, boundary scheduler only.

The first JS artifact was measured only after the cost-model deletions below, then once more after the `from_utf8` crash fix. Incremental sizes during the rewrite were not collected because each rustc-codegen-js link is minutes; the one rebuild that did ship is recorded.

## Deletions

### 1. Fiber scheduler and `fingerprint_fiber`

Deleted the fiber variant, `collection_break`/`source_checkpoint` cfg splits, fiber canvas phases, and fiber scripts in `tools/build.mjs`. `dist/` is a single boundary build.

Size: not isolated. Fiber was already a few KB larger than boundary in the idiomatic experiment.

### 2. `format!` / Debug / Display / float formatter

Removed every product `format!`, `to_string()` of floats, `Display`, and `Debug`.

- `print_debug` and the confidence comment now assemble strings on the host (`join_as_strings` + `String(number)`).
- Color-gamut queries are static. Monochrome queries are host concat of a static prefix and `String(depth)`.
- Timezone fallback, font-family strings, selector errors, blocker debug, and zoom are host concat.
- Typed helpers that existed only for native unit tests (`canonical.rs`, `confidence.rs`, `sources_sync.rs`, the CSS `Display` parser) are no longer compiled into the product crate.

`flt2dec` is gone from `dist/raw/program.js`. That was the named ~125 KB debug-float cluster.

First shipped minified ESM after this group (plus the other deletions): **195,996 B**.

### 3. `str::contains` / Unicode-facing str methods

Replaced with byte scans:

- cookie test: `bytes_contains(..., b"cookietest=")`
- Android `appVersion`: ASCII case-insensitive scan for `b"android"`
- Apple Pay "from"/"insecure": ASCII word-boundary scan, no `to_ascii_lowercase()` allocation
- CSS declaration trim: ASCII whitespace only

`str::contains` itself is not in the product crate. Some Unicode tables remain reachable through `str` indexing / panic formatting (see remaining core).

### 4. Coroutine state machines

No `async`/`await`, no `future_to_promise`, no `await_promise`, no `yield_task`.

`mapWithBreaks` is a host Promise chain: 16 ms `Date.now()` budget, `MessageChannel` yield (`yield_to_task`), per-item callback is not awaited, then `Promise.all`. `load`, `withIframe`, fonts, blockers, font preferences, screen frame, and UA-CH use `.then` / `callback0` / `callback1`.

Optimizer report after the final build: no `core::future` / coroutine poller items. The old 11.3% raw coroutine tax is gone.

### 5. Murmur unboxed

`X64 { high, low }` is gone. Lanes are four `u32` parameters. 16-bit helpers are `#[inline(always)]`. Blocks are read with shifts, not `from_le_bytes` on a copied array. Digest is a `[u8; 32]` of ASCII hex.

### 6. Owned host `String` / `Vec` / generic component types

Components stay `JsValue`. Hashing never takes ownership of the host input string (`input.read::<&str>()`). Canonicalization, debug serialization, and hex emission stay on the host.

`load_sources` no longer buffers getters in a Rust `Vec`. `try_call` no longer `to_vec()`s arguments.

`alloc` stays. `Rc`/`RefCell` (audio render, iframe load, screen-frame state, `mapWithBreaks`) and a small `Vec` for blocker-selector attributes still need it. Removing `alloc` would force rewriting those into host objects with no measured win in this pass.

One component type: `JsValue`. No generic Result family in the product path.

### 7. `str::from_utf8` on the hex digest (post-first-build)

The first artifact crashed in Chrome/Firefox/Safari:

```
rustc_codegen_js: this pointer is not a byte view of 4-byte elements
  at run_utf8_validation / from_utf8 / hash_hex
```

`from_utf8` on `[u8; 32]` is a backend limitation, not a contract change. Hex is now 32 static digit `&str`s joined on the host.

| Artifact | Before | After |
| --- | ---: | ---: |
| `dist/raw/program.js` | 625,972 | 614,026 |
| `dist/fp.esm.min.js` | 195,996 | 192,850 |
| reachable `core` | 110,111 | 96,807 |

That also dropped `run_utf8_validation` (~12 KB).

## Stopped here

Further product deletions that would need a compiler change, or that risk the public contract:

- `panic_fmt` / `pad_integral` / `char::Debug` / Unicode tables still come from slice/`str` bounds checks (`slice_error_fail_rt`, `floor_char_boundary`). Killing those means a panic-without-formatting core or never indexing `&str`.
- Host boundaries still block native `String`/`Vec` regions (planner declines in the report).
- `alloc` `Vec` grow for blocker attributes.

## Remaining reachable core (final optimizer report)

`core` is 76 items / 96,807 B of a 614,026 B raw program. Largest:

| Bytes | Item |
| ---: | --- |
| 10,139 | `str::slice_error_fail_rt` |
| 10,074 | `fmt::write` |
| 6,165 | `fmt::Formatter::pad_integral` |
| 5,748 | `char::escape_debug_ext` |
| 5,067 | `unicode_data::cn_planes_0_3::OFFSETS` |
| 4,889 | `fmt::num` usize `_fmt_inner` |
| 4,196 | `str::count::do_count_chars` |
| 2,676 | `escape::escape_unicode` |
| 2,659 | `unicode_data::grapheme_extend::OFFSETS` |

`alloc` is 11 items / 12,580 B. No `flt2dec`. No Future poller.

Crate split of the raw program: app 495,425 · core 96,807 · alloc 12,580 · entry 5,333 · mires_js 3,053.
