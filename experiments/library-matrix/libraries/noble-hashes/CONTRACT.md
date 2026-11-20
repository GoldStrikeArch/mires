# @noble/hashes 2.4.0

The selected API includes SHA-256, SHA-512, SHA3-256, Keccak-256, BLAKE2b, BLAKE2s and HMAC-SHA-256. Each operation returns its complete lowercase hexadecimal digest. SHA3-256 and Keccak-256 use different domain separators. All hashing runs in the port language.

Each input names an `algorithm` and supplies either a one-shot `bytes` array or a `chunks` array for incremental `create().update().digest()`. Every input byte is an integer from 0 through 255. Zero-length messages and empty updates are included. The optional positive integer `repeat` repeats the chunk sequence on the same hash state. The return value is a string. Input arrays keep their original contents.

HMAC takes a required byte-array `key`, including an empty key. BLAKE2 accepts optional `dkLen`, `key`, `salt` and `personalization`. For BLAKE2b the digest and present keys range from 1 to 64 bytes, while salt and personalization have exactly 16 bytes. BLAKE2s uses limits of 32 and 8 bytes respectively. A fractional digest length, out-of-range length, or unknown algorithm throws. Each call starts with fresh state.

The three benchmark cases hash exactly 1 MiB each, in 256 updates of a 4,096-byte block. The variant changes four bytes of that block while preserving the number of compression rounds. The canonical reference calls the pinned npm package directly.

The corpus covers block and padding boundaries, HMAC key normalization, domain separation, short keyed BLAKE2 digests, invalid parameters, and incremental chunk splits. Run `node generate-corpus.mjs` here to regenerate it. Run `node verify.mjs <language>` after building a port for the digest vectors and a length-by-length comparison.

`known-answers.json` contains selected byte-oriented CAVP short-message vectors, keyed and unkeyed BLAKE2 answer vectors, the empty Keccak-256 digest, and RFC 4231 HMAC-SHA-256 test case 1. [NIST SHA-2 vectors](https://csrc.nist.gov/CSRC/media/Projects/Cryptographic-Algorithm-Validation-Program/documents/shs/shabytetestvectors.zip) · [NIST SHA-3 vectors](https://csrc.nist.gov/CSRC/media/Projects/Cryptographic-Algorithm-Validation-Program/documents/sha3/sha-3bytetestvectors.zip) · [BLAKE2 test vectors](https://github.com/BLAKE2/BLAKE2/blob/master/testvectors/blake2-kat.json) · [RFC 4231](https://www.rfc-editor.org/rfc/rfc4231#section-4.2)
