# Rust hash port

Each call owns its SHA-2, Keccak, BLAKE2 or HMAC state. The incremental path retains the partial block across updates and applies padding only when the caller requests the digest. SHA-512, BLAKE2b and Keccak represent each 64-bit lane as two 32-bit words.

`WORKAROUND(MIRES-020)` in `src/sha2.rs` passes the SHA-512 state slice to a named function. The original capturing closure returning the word pair fails in mires during call lowering. The [standalone reproducer](../../../mires-bugs/MIRES-020-closure-spread-cleanup/) includes the expected compiler diagnostic.
