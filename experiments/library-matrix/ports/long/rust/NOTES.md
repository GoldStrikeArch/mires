# long in Rust

The port uses wrapping Rust integers for the native 64-bit arithmetic path proposed in the candidate research. The pinned library's signed WebAssembly division guard is reproduced explicitly. Parsing handles eight characters at a time, including the JavaScript prefix and whitespace behavior.

The parsing-errors corpus exposed MIRES-019 independently in both the leading plus test and the hexadecimal prefix test. Comparing an indexed byte to the promoted constant `Some(&b'+')` compiled the constant as `None`, making `"  +42"` parse as zero. Comparing copied byte values fixes both tests. [Compiler repro](../../../mires-bugs/MIRES-019-option-ref-const-decodes-none/)
