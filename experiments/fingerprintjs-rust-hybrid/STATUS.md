# Hybrid Rust+JS FingerprintJS 5.2.0

| Gate | Result |
| --- | --- |
| Build | pass (`js-opt=on`, boundary only) |
| Artifacts | `dist/fp.esm.js`, `dist/fp.esm.min.js`, `dist/fp.min.js`, `dist/fp.d.ts`, `dist/attribution.json` |
| Types | pass (`fp.d.ts` SHA-256 `d20e2e6b3f60afb7679c1f87dd62800b7a7ce7155e86efbf6206190a1c185731`) |
| SSR import | pass |
| Package exports | pass (named + default identity + 42 sources) |
| Parity | complete — 0 normalized diffs on Chrome / Firefox / Safari |
| Size | fail vs oracle gates (honest rustc-codegen-js result) |
| Mostly JS | no — `jsKernels` is 3.7% of the minified IIFE |

`unsafe_code = "deny"` still. Fiber scheduler is gone. Algorithm version stays `5.2.0`.

## Size vs oracle gates

| Artifact | bytes | gzip-9 | brotli-11 | oracle bytes |
| --- | ---: | ---: | ---: | ---: |
| `fp.min.js` | 269643 | 68646 | 56222 | 36746 |
| `fp.umd.min.js` | 269993 | 68719 | 56221 | 36937 |

## Attribution (`dist/attribution.json`)

```json
{
  "rustProgram": 907634,
  "rustShim": 22492,
  "jsKernels": 10020,
  "facade": 2687
}
```

`jsKernels` is the sum of `js/*.js` plus the one `js!{}` body (`canvas.toDataURL()`). It is not mostly JS.
