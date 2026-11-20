# lz-string 1.5.0

The matrix exercises the ten public compression and decompression methods in the package's five formats. The `raw`, `base64`, `utf16`, `uri` and `bytes` names select the unadorned methods and their Base64, UTF16, EncodedURIComponent and Uint8Array variants.

Inputs have an `op`, a `format` and `data`. JSON represents strings as arrays of UTF-16 code units, preserving isolated surrogates and NUL. The Uint8Array representation uses unsigned bytes in big-endian order. Results preserve every element, null result and exception. Null inputs and empty inputs have separate cases. Decoders accept the published wire format independently of the port's encoder.

The deterministic corpus includes examples from the published test suite, mixed scripts, byte-width and dictionary-width boundaries, surrogate halves, truncated streams and odd byte lengths. The benchmark compresses about 100,000 code units of newline-delimited JSON to Base64, decompresses it, checks every unit and returns the two lengths. The variant bijectively renames ASCII letters throughout the record while preserving its JSON syntax, length and symbol repetition pattern.

The reference uses the original UMD source published as `lz-string@1.5.0`. The harness postinstall step wraps that source as an ES module so the browser and Node use the same package code.
