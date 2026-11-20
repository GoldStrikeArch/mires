# Rust diff port

The Myers search keeps the furthest old-file position on each diagonal and shares edit histories through a per-call arena. The separate tokenizers preserve Unicode code points, duplicated whitespace around word tokens, line endings, and typed array elements. Patch generation, parsing, reversal, exact hunk fitting and offset search run in Rust. The boundary uses JavaScript property access and JSON encoding for JSON input, and keeps the package's property insertion order and explicit `undefined` values on output.

Two previously tracked compiler defects recurred during the port.

- `WORKAROUND(MIRES-019)` in `src/patch.rs` compares a copied opening quote and queries the final source line directly. Promoted `Some(&b'"')` and `Some(&"")` comparisons produced incorrect results for quoted file names and EOF-newline removal. [Promoted-reference reproducer](../../../mires-bugs/MIRES-019-option-ref-const-decodes-none/)
- `WORKAROUND(MIRES-017)` in `src/patch.rs` decodes the validated UTF-8 prefix and emits a replacement character for each invalid sequence. The original `String::from_utf8_lossy` made a valid quoted name throw on the return path of `Utf8Chunks::next`. The generated switch assigned the successful result and fell through into its default `bad block 1` exception. [Structurizer reproducer](../../../mires-bugs/MIRES-017-switch-last-case-missing-break/)
