# `str::to_ascii_lowercase` and `make_ascii_lowercase` leave the text unchanged (silent miscompile)

`port/src/lib.rs` converts the input text with `str::to_ascii_lowercase()` and, on an owned copy,
with `String::make_ascii_uppercase()`, and compares each result with the same conversion done one
byte at a time (`u8::to_ascii_lowercase`, `u8::to_ascii_uppercase`). The build passes with no
warning. On any text with an ASCII letter to change, the call returns:

```text
MISCOMPILED: str::to_ascii_lowercase returned the text unchanged
```

Native Rust lowercases and uppercases the ASCII letters.

All four ASCII case conversions of `str` are affected: `to_ascii_lowercase`, `to_ascii_uppercase`,
`make_ascii_lowercase` and `make_ascii_uppercase`. Each changes the bytes through `&mut str`
(`to_ascii_*` copies the text and calls `make_ascii_*` on the copy, which converts
`str::as_bytes_mut`). In a scratch probe on `"The Quick BROWN fox"`, all four returned the text
unchanged, while `Vec<u8>::make_ascii_lowercase`, `[u8]::to_ascii_lowercase`,
`chars().map(char::to_ascii_lowercase)` and `str::to_lowercase` returned
`"the quick brown fox"`.

Expected: `run({ text })` returns `text` for every floor case. Actual: `ascii` and `unicode`
return the message above, and so does `empty` on its rewritten call, whose text has letters.

Reproduce: `./run.sh`. The build passes, the check fails on all three floor cases, and the script
prints the port's answer for `"The Quick Brown Fox"`.

Settings: `js-profile=release-size js-opt=on js-ecma-target=2018 js-cleanup=off`, unwind mode
(`JSC_PANIC` unset), `JSC_RUST_OPT_LEVEL=z`, `JSC_RUST_MIR_INLINE=off`, `js-lane=jir`, through
`experiments/library-matrix/languages/rust/build.mjs`; compiler
`f525806d7907ab0f85c8c37a873419d3565da753` (the checkout was at `174279a`, which changes no compiler
source since `f525806`).

## Impact

Found by the ky Rust port. Its first version normalised HTTP methods with `to_ascii_lowercase` and
`to_ascii_uppercase`, so no common method matched Ky's retry list and no request retried.

Workaround in the port: `ports/ky/rust/src/ky.rs` (`normalize_request_method`,
`normalize_retry_method`) uses `to_lowercase` and `to_uppercase`, tagged
`// WORKAROUND(MIRES-025)`. They are JS's `toLowerCase` and `toUpperCase` mappings, which Ky uses,
so they give Ky's results on every input; the ASCII forms give the same results on every method
name Ky matches.
