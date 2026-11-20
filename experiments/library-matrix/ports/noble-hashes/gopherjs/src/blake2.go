package main

// BLAKE2s and BLAKE2b use the same schedule with different word widths.
var blakeHigh = [8]uint32{0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19}
var blakeLow = [8]uint32{0xf3bcc908, 0x84caa73b, 0xfe94f82b, 0x5f1d36f1, 0xade682d1, 0x2b3e6c1f, 0xfb41bd6b, 0x137e2179}
var blakeSigma = [10][16]int{
	{0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15},
	{14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3},
	{11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4},
	{7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8},
	{9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13},
	{2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9},
	{12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11},
	{13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10},
	{6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5},
	{10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0},
}

type blakeWord struct{ lo, hi uint32 }

func (a blakeWord) add(b blakeWord) blakeWord {
	low := a.lo + b.lo
	hi := a.hi + b.hi
	if low < a.lo {
		hi++
	}
	return blakeWord{low, hi}
}
func (a blakeWord) xor(b blakeWord) blakeWord { return blakeWord{a.lo ^ b.lo, a.hi ^ b.hi} }
func (w blakeWord) rotate(n uint, wide bool) blakeWord {
	if !wide {
		return blakeWord{lo: w.lo>>n | w.lo<<(32-n)}
	}
	if n == 32 {
		return blakeWord{w.hi, w.lo}
	}
	l, h := w.lo, w.hi
	if n > 32 {
		l, h, n = h, l, n-32
	}
	return blakeWord{l>>n | h<<(32-n), h>>n | l<<(32-n)}
}
func blakeIV(i int, wide bool) blakeWord {
	if wide {
		return blakeWord{blakeLow[i], blakeHigh[i]}
	}
	return blakeWord{lo: blakeHigh[i]}
}
func blakeLoad32(b []byte, i int) uint32 {
	return uint32(b[i]) | uint32(b[i+1])<<8 | uint32(b[i+2])<<16 | uint32(b[i+3])<<24
}
func blakeLoad(b []byte, i int, wide bool) blakeWord {
	if wide {
		return blakeWord{blakeLoad32(b, 8*i), blakeLoad32(b, 8*i+4)}
	}
	return blakeWord{lo: blakeLoad32(b, 4*i)}
}
func blakeMix(v []blakeWord, a, b, c, d int, x, y blakeWord, wide bool) {
	r1, r2, r3, r4 := uint(16), uint(12), uint(8), uint(7)
	if wide {
		r1, r2, r3, r4 = 32, 24, 16, 63
	}
	v[a] = v[a].add(v[b]).add(x)
	v[d] = v[d].xor(v[a]).rotate(r1, wide)
	v[c] = v[c].add(v[d])
	v[b] = v[b].xor(v[c]).rotate(r2, wide)
	v[a] = v[a].add(v[b]).add(y)
	v[d] = v[d].xor(v[a]).rotate(r3, wide)
	v[c] = v[c].add(v[d])
	v[b] = v[b].xor(v[c]).rotate(r4, wide)
}
func blakeCompress(h []blakeWord, block []byte, count blakeWord, last, wide bool) {
	var v, m [16]blakeWord
	for i := 0; i < 8; i++ {
		v[i], v[i+8] = h[i], blakeIV(i, wide)
	}
	for i := 0; i < 16; i++ {
		m[i] = blakeLoad(block, i, wide)
	}
	v[12].lo ^= count.lo
	if wide {
		v[12].hi ^= count.hi
	} else {
		v[13].lo ^= count.hi
	}
	if last {
		v[14].lo, v[14].hi = ^v[14].lo, ^v[14].hi
	}
	rounds := 10
	if wide {
		rounds = 12
	}
	for r := 0; r < rounds; r++ {
		s := blakeSigma[r%10][:]
		for i := 0; i < 4; i++ {
			blakeMix(v[:], i, i+4, i+8, i+12, m[s[2*i]], m[s[2*i+1]], wide)
		}
		for i := 0; i < 4; i++ {
			blakeMix(v[:], i, 4+(i+1)%4, 8+(i+2)%4, 12+(i+3)%4, m[s[8+2*i]], m[s[9+2*i]], wide)
		}
	}
	for i := range h {
		h[i] = h[i].xor(v[i]).xor(v[i+8])
	}
}
func blake2Digest(wide bool, chunks [][]byte, dkLen int, key, salt, personalization []byte) []byte {
	stride := 4
	if wide {
		stride = 8
	}
	blockLen := 16 * stride
	var h [8]blakeWord
	for i := range h {
		h[i] = blakeIV(i, wide)
	}
	h[0].lo ^= 0x01010000 | uint32(len(key))<<8 | uint32(dkLen)
	for i := 0; i < 2; i++ {
		if len(salt) > 0 {
			h[i+4] = h[i+4].xor(blakeLoad(salt, i, wide))
		}
		if len(personalization) > 0 {
			h[i+6] = h[i+6].xor(blakeLoad(personalization, i, wide))
		}
	}
	block := make([]byte, blockLen)
	copy(block, key)
	pos := 0
	if len(key) > 0 {
		pos = blockLen
	}
	count := blakeWord{lo: uint32(pos)}
	for _, chunk := range chunks {
		for len(chunk) > 0 {
			if pos == blockLen {
				blakeCompress(h[:], block, count, false, wide)
				pos = 0
			}
			take := copy(block[pos:], chunk)
			count = count.add(blakeWord{lo: uint32(take)})
			pos += take
			chunk = chunk[take:]
		}
	}
	for i := pos; i < blockLen; i++ {
		block[i] = 0
	}
	blakeCompress(h[:], block, count, true, wide)
	out := make([]byte, dkLen)
	for i := range out {
		w := h[i/stride]
		limb := w.lo
		if i%stride >= 4 {
			limb = w.hi
		}
		out[i] = byte(limb >> uint(8*(i%4)))
	}
	return out
}
