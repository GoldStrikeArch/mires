# Standalone Rust-to-JavaScript experiments

These are restored Rust workloads and the OSS reference, not recovered October performance results. Install tools at the repository root with `npm ci --ignore-scripts`.

```sh
npm run build:oss-reference
node experiments/fingerprintjs-rust-compact/tools/build.mjs --profile release-balanced --optimizer on --lane jir --cleanup on --scheduler boundary --output experiments/fingerprintjs-rust-compact/dist/mires-v2
node experiments/fingerprintjs-rust/tools/build.mjs --profile release-balanced --optimizer on --lane jir --cleanup on --scheduler boundary --output experiments/fingerprintjs-rust/dist/mires-v2
npm run build:corpus
npm run test:corpus
```

Use explicit `--cleanup off` only for a labeled diagnostic comparison. All runtime/reference comparisons must name the built artifacts; archived reports describe their own historical inputs. Fingerprint Pro is restored separately under `experiments-pro/` and kept private. Other-language ports have not been restored.

## Restored other-language ports

All eight non-Rust OSS comparator families (GopherJS was added on 2026-11-18) build in the standalone checkout. Run `npm run build:comparators -- --suite oss` and `npm run check:comparators -- --suite oss` from the repository root. See [the current guide](../docs/COMPARATORS.md). Old `STATUS.md` files and archived matrices are historical, not current qualification.
