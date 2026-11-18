package main

import (
	"math"
	"math/bits"

	"github.com/gopherjs/gopherjs/js"
)

// MurmurHash3 x64 128, as FingerprintJS uses it. The oracle emulates the 64-bit lanes with
// pairs of 32-bit numbers; the port uses Go's uint64, which GopherJS emulates in turn.

const (
	murmurC1 = 0x87c37b91114253d5
	murmurC2 = 0x4cf5ad432745937f
)

func fmix64(k uint64) uint64 {
	k ^= k >> 33
	k *= 0xff51afd7ed558ccd
	k ^= k >> 33
	k *= 0xc4ceb9fe1a85ec53
	k ^= k >> 33
	return k
}

// littleEndian64 reads up to eight bytes as a little-endian number.
func littleEndian64(b []byte) uint64 {
	var v uint64
	for i := len(b) - 1; i >= 0; i-- {
		v = v<<8 | uint64(b[i])
	}
	return v
}

// x64hash128 returns the 128-bit MurmurHash3 x64 hash of key as 32 hex digits.
func x64hash128(key []byte, seed uint32) string {
	h1, h2 := uint64(seed), uint64(seed)
	length := len(key)
	blocks := length - length%16

	for i := 0; i < blocks; i += 16 {
		k1 := littleEndian64(key[i : i+8])
		k2 := littleEndian64(key[i+8 : i+16])

		k1 *= murmurC1
		k1 = bits.RotateLeft64(k1, 31)
		k1 *= murmurC2
		h1 ^= k1
		h1 = bits.RotateLeft64(h1, 27)
		h1 += h2
		h1 = h1*5 + 0x52dce729

		k2 *= murmurC2
		k2 = bits.RotateLeft64(k2, 33)
		k2 *= murmurC1
		h2 ^= k2
		h2 = bits.RotateLeft64(h2, 31)
		h2 += h1
		h2 = h2*5 + 0x38495ab5
	}

	if tail := key[blocks:]; len(tail) > 0 {
		if len(tail) > 8 {
			k2 := littleEndian64(tail[8:])
			k2 *= murmurC2
			k2 = bits.RotateLeft64(k2, 33)
			k2 *= murmurC1
			h2 ^= k2
		}
		k1 := littleEndian64(tail[:min(len(tail), 8)])
		k1 *= murmurC1
		k1 = bits.RotateLeft64(k1, 31)
		k1 *= murmurC2
		h1 ^= k1
	}

	h1 ^= uint64(length)
	h2 ^= uint64(length)
	h1 += h2
	h2 += h1
	h1 = fmix64(h1)
	h2 = fmix64(h2)
	h1 += h2
	h2 += h1

	return hex64(h1) + hex64(h2)
}

func hex64(v uint64) string {
	const digits = "0123456789abcdef"
	var b [16]byte
	for i := len(b) - 1; i >= 0; i-- {
		b[i] = digits[v&0xf]
		v >>= 4
	}
	return string(b[:])
}

// utf8Bytes is the oracle's getUTF8Bytes: the UTF-8 encoding of a JavaScript string. Other
// values get the oracle's walk: one byte per charCodeAt over input.length, or TextEncoder
// once a code exceeds 127 (an object without length gives no bytes; null throws).
func utf8Bytes(input *js.Object) []byte {
	if typeOf(input) == "string" {
		return []byte(utf8String(input))
	}
	result := js.Global.Get("Uint8Array").New(input.Get("length"))
	for i := 0; float64(i) < toNumber(input.Get("length")); i++ {
		charCode := input.Call("charCodeAt", i)
		if charCode.Float() > 127 {
			return textEncode(input)
		}
		result.SetIndex(i, charCode)
	}
	return byteSlice(result)
}

// hashSeed converts a JavaScript seed the way the oracle's 32-bit arithmetic does:
// `seed || 0`, then ToUint32.
func hashSeed(seed *js.Object) uint32 {
	if !seed.Bool() {
		return 0
	}
	return toUint32(toNumber(seed))
}

// toUint32 is ECMAScript ToUint32: truncate, then wrap modulo 2^32. The integer part comes
// from math.Modf: GopherJS implements math.Trunc through a 32-bit int, which wraps beyond
// ±2^31 (math.Trunc(4294967295) is 1).
func toUint32(f float64) uint32 {
	if math.IsNaN(f) || math.IsInf(f, 0) {
		return 0
	}
	whole, _ := math.Modf(math.Mod(f, 1<<32))
	if whole < 0 {
		whole += 1 << 32
	}
	return uint32(whole)
}

// murmurX64Hash128 is the exported x64hash128(input, seed).
func murmurX64Hash128(input, seed *js.Object) string {
	return x64hash128(utf8Bytes(input), hashSeed(seed))
}
