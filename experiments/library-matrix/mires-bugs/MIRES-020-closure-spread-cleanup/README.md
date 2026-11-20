# MIRES-020

A capturing closure reads a pair of words by index. Its caller later consumes an array of the resulting pairs with an owning iterator. With the matrix Rust release-size profile on compiler `f525806`, the linker refuses the caller with `requires semantic cleanup lowering` and `SpreadNotTuple` at a call terminator. A native build returns 10 for input 0 and 14 for input 1.

Run `./run.sh` to build the isolated fixture. It succeeds only when the linker emits the recorded diagnostic. The SHA-512 port uses a named function taking the slice explicitly.
