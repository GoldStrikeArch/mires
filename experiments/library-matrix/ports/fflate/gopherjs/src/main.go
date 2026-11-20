// Package main is fflate 0.8.3 in Go, compiled to JavaScript by GopherJS: a faithful port of the
// synchronous compression, decompression, framing, streaming, and text-codec subset defined in
// libraries/fflate/CONTRACT.md.
//
//   - tables.go: immutable module-load lookup tables matching fflate's precomputed tables;
//   - huffman.go: hMap, hTree (flat primitive arrays, UZIP.js linear merge, 15-bit/7-bit Kraft
//     debt clamping and rollback), lc, clen;
//   - deflate.go: wbits, wbits16, wfblk, wblk, dflt, dopt, deflateSync;
//   - inflate.go: readBits, readBits16, inflt, inflateSync;
//   - framing.go: CRC-32, Adler-32, wbytes, GZIP/Zlib framing, decompressSync, FlateError codes;
//   - stream.go: Deflate, Gzip, Zlib, Inflate, Gunzip, Unzlib, Decompress, and concrete streamSink;
//   - codec.go: WHATWG UTF-8 and Latin-1 strFromU8 -> UTF-16LE bytes;
//   - boundary.go: corpus input decoding, dispatch, error-code framing, zero-copy Uint8Array return.
//
// run is published on js.Module.exports, which the builder's facade binds and re-exports.
package main

import "github.com/gopherjs/gopherjs/js"

func main() {
	js.Module.Get("exports").Set("run", run)
}
