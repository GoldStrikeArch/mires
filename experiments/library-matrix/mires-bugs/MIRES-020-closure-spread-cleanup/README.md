# MIRES-020

A capturing closure reads a pair of words by index. Its caller later consumes an array of the resulting pairs with an owning iterator. With the matrix Rust release-size profile on compiler `f525806`, the linker refuses the caller with `requires semantic cleanup lowering` and `SpreadNotTuple` at a call terminator. A native build returns 10 for input 0 and 14 for input 1.

The call `unpack(1)` passes its argument tuple as one constant, `const (1_usize,)`, once `-Copt-level=z` has folded it. A tuple whose layout is a single scalar evaluates to that scalar rather than to memory, and the JIR encoder only split constant tuples that were memory. It now puts the scalar back into a scratch allocation laid out for the tuple and decodes the tuple from there.

Run `./run.sh` to build the isolated fixture and check it against the floor corpus. The port returns the input text unchanged when `sum_lanes` gives the native answers for both indices, so the check passes exactly when the closure call is compiled correctly. The script expects build PASS and check PASS; on a compiler with the bug the build fails with `reason: SpreadNotTuple` and the script exits non-zero. The SHA-512 port uses a named function taking the slice explicitly.
