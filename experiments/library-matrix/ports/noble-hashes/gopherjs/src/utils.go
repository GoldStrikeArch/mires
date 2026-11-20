package main

import "math"

// hashError is the error noble throws for an invalid option or algorithm.
type hashError string

func (e hashError) Error() string { return string(e) }

// hash is noble's Hash: an incremental hash with its block and digest lengths.
type hash interface {
	blockLen() int
	outputLen() int
	update(data []byte)
	digest() []byte
}

// anumber is noble's: n must be a safe integer >= 0. Values above math.MaxInt32 saturate.
func anumber(n float64) (int, error) {
	if !(n >= 0 && n <= 1<<53-1 && n == math.Trunc(n)) {
		return 0, hashError("expected integer >= 0")
	}
	return int(min(n, math.MaxInt32)), nil
}

// getUint32BE is DataView.getUint32(offset, false).
func getUint32BE(b []byte, offset int) uint32 {
	return uint32(b[offset])<<24 | uint32(b[offset+1])<<16 | uint32(b[offset+2])<<8 | uint32(b[offset+3])
}

// getUint32LE reads a little-endian word, as noble's Uint32Array views do on little-endian hosts.
func getUint32LE(b []byte, offset int) uint32 {
	return uint32(b[offset]) | uint32(b[offset+1])<<8 | uint32(b[offset+2])<<16 | uint32(b[offset+3])<<24
}

// putUint32BE is DataView.setUint32(offset, v, false).
func putUint32BE(b []byte, offset int, v uint32) {
	b[offset] = byte(v >> 24)
	b[offset+1] = byte(v >> 16)
	b[offset+2] = byte(v >> 8)
	b[offset+3] = byte(v)
}

// bytesToHex is noble's.
func bytesToHex(b []byte) string {
	const digits = "0123456789abcdef"
	out := make([]byte, 2*len(b))
	for i, v := range b {
		out[2*i] = digits[v>>4]
		out[2*i+1] = digits[v&15]
	}
	return string(out)
}
