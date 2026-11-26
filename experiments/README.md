# Experiments

This directory holds the FingerprintJS workloads, the multi-library compiler matrix, and the non-Rust comparator ports. Install root Node dependencies once from the repository root with `npm ci --ignore-scripts`.

## Rust FingerprintJS and corpus builds

Mires v5 uses unwind mode by default, where `rustc` preserves cleanup edges natively, so pass `--cleanup off` alongside `--lane jir`:

```sh
npm run build:oss-reference
node experiments/fingerprintjs-rust-compact/tools/build.mjs \
  --profile release-size --optimizer on --lane jir --cleanup off --scheduler boundary \
  --output experiments/fingerprintjs-rust-compact/dist/mires-v5
node experiments/fingerprintjs-rust/tools/build.mjs \
  --profile release-size --optimizer on --lane jir --cleanup off --scheduler boundary \
  --output experiments/fingerprintjs-rust/dist/mires-v5
npm run build:corpus
npm run test:corpus
```

Always label comparisons with the exact built artifacts and compiler options; older `STATUS.md` and `VERDICT.md` files inside port directories record historical runs. Fingerprint Pro lives in `experiments-pro/` and is kept private (gitignored).

## Library matrix

[`library-matrix/README.md`](library-matrix/README.md) documents the 15-library (+ Explorer, `pilot`, and `floor`) cross-language parity, size, and timing harness covering Mires and the eight other compile-to-JS languages.

## Other-language comparators

All eight non-Rust OSS comparator families (`rescript`, `melange`, `jsoo`, `scalajs`, `dart`, `kotlin`, `elm`, and `gopherjs`) build from this checkout:

```sh
npm run build:comparators -- --suite oss
npm run check:comparators -- --suite oss
```

See [docs/COMPARATORS.md](../docs/COMPARATORS.md) for toolchain setup, production artifact paths, and known compiler caveats.
