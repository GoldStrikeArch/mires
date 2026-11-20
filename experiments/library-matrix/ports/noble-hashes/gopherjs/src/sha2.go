package main

import "math/bits"

// noble's sha2.ts: SHA-256 (SHA2_32B) and SHA-512 (SHA2_64B), with the IVs and the HashMD
// buffering of _md.ts. Go has no base classes, so each hash carries HashMD's update and digest;
// sharing them through an interface or a function value would make GopherJS compile every caller
// into a resumable state machine.

var (
	sha256IV = [8]uint32{
		0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
	}

	sha256K = [64]uint32{
		0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
		0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
		0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
		0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
		0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
		0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
		0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
		0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
	}

	sha512IV = [16]uint32{
		0x6a09e667, 0xf3bcc908, 0xbb67ae85, 0x84caa73b, 0x3c6ef372, 0xfe94f82b, 0xa54ff53a, 0x5f1d36f1,
		0x510e527f, 0xade682d1, 0x9b05688c, 0x2b3e6c1f, 0x1f83d9ab, 0xfb41bd6b, 0x5be0cd19, 0x137e2179,
	}

	// The 80 round constants of RFC 6234 §5.2, split into high and low halves as noble's K512 is.
	sha512Kh = [80]uint32{
		0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
		0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
		0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
		0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
		0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
		0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
		0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
		0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
		0xca273ece, 0xd186b8c7, 0xeada7dd6, 0xf57d4f7f, 0x06f067aa, 0x0a637dc5, 0x113f9804, 0x1b710b35,
		0x28db77f5, 0x32caab7b, 0x3c9ebe0a, 0x431d67c4, 0x4cc5d4be, 0x597f299c, 0x5fcb6fab, 0x6c44198c,
	}
	sha512Kl = [80]uint32{
		0xd728ae22, 0x23ef65cd, 0xec4d3b2f, 0x8189dbbc, 0xf348b538, 0xb605d019, 0xaf194f9b, 0xda6d8118,
		0xa3030242, 0x45706fbe, 0x4ee4b28c, 0xd5ffb4e2, 0xf27b896f, 0x3b1696b1, 0x25c71235, 0xcf692694,
		0x9ef14ad2, 0x384f25e3, 0x8b8cd5b5, 0x77ac9c65, 0x592b0275, 0x6ea6e483, 0xbd41fbd4, 0x831153b5,
		0xee66dfab, 0x2db43210, 0x98fb213f, 0xbeef0ee4, 0x3da88fc2, 0x930aa725, 0xe003826f, 0x0a0e6e70,
		0x46d22ffc, 0x5c26c926, 0x5ac42aed, 0x9d95b3df, 0x8baf63de, 0x3c77b2a8, 0x47edaee6, 0x1482353b,
		0x4cf10364, 0xbc423001, 0xd0f89791, 0x0654be30, 0xd6ef5218, 0x5565a910, 0x5771202a, 0x32bbd1b8,
		0xb8d2d0c8, 0x5141ab53, 0xdf8eeb99, 0xe19b48a8, 0xc5c95a63, 0xe3418acb, 0x7763e373, 0xd6b2b8a3,
		0x5defb2fc, 0x43172f60, 0xa1f0ab72, 0x1a6439ec, 0x23631e28, 0xde82bde9, 0xb2c67915, 0xe372532b,
		0xea26619c, 0x21c0c207, 0xcde0eb1e, 0xee6ed178, 0x72176fba, 0xa2c898a6, 0xbef90dae, 0x131c471b,
		0x23047d84, 0x40c72493, 0x15c9bebc, 0x9c100d4c, 0xcb3e42b6, 0xfc657e2a, 0x3ad6faec, 0x4a475817,
	}
)

// chi returns the bits of b where a is set, else of c.
func chi(a, b, c uint32) uint32 { return a&b ^ ^a&c }

// maj returns the majority of each bit of a, b and c.
func maj(a, b, c uint32) uint32 { return a&b ^ a&c ^ b&c }

// rotr is noble's 32-bit right rotation.
func rotr(word uint32, shift int) uint32 { return bits.RotateLeft32(word, -shift) }

// sha256 is noble's SHA2_32B with the SHA-256 IV.
type sha256 struct {
	a, b, c, d, e, f, g, h uint32
	w                      [64]uint32 // the message schedule; noble shares one module-level SHA256_W
	buffer                 [64]byte
	pos                    int
	length                 uint64
}

func newSHA256() *sha256 {
	iv := sha256IV
	return &sha256{a: iv[0], b: iv[1], c: iv[2], d: iv[3], e: iv[4], f: iv[5], g: iv[6], h: iv[7]}
}

func (s *sha256) blockLen() int  { return 64 }
func (s *sha256) outputLen() int { return 32 }

func (s *sha256) process(data []byte, offset int) {
	w := &s.w
	for i := 0; i < 16; i++ {
		w[i] = getUint32BE(data, offset+4*i)
	}
	for i := 16; i < 64; i++ {
		w15, w2 := w[i-15], w[i-2]
		s0 := rotr(w15, 7) ^ rotr(w15, 18) ^ w15>>3
		s1 := rotr(w2, 17) ^ rotr(w2, 19) ^ w2>>10
		w[i] = s1 + w[i-7] + s0 + w[i-16]
	}
	a, b, c, d, e, f, g, h := s.a, s.b, s.c, s.d, s.e, s.f, s.g, s.h
	for i := 0; i < 64; i++ {
		sigma1 := rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25)
		t1 := h + sigma1 + chi(e, f, g) + sha256K[i] + w[i]
		sigma0 := rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22)
		t2 := sigma0 + maj(a, b, c)
		h, g, f, e, d, c, b, a = g, f, e, d+t1, c, b, a, t1+t2
	}
	s.a += a
	s.b += b
	s.c += c
	s.d += d
	s.e += e
	s.f += f
	s.g += g
	s.h += h
}

func (s *sha256) update(data []byte) {
	n := len(data)
	for pos := 0; pos < n; {
		take := min(64-s.pos, n-pos)
		// With nothing buffered, whole blocks are compressed straight from the input.
		if take == 64 {
			for ; 64 <= n-pos; pos += 64 {
				s.process(data, pos)
			}
			continue
		}
		copy(s.buffer[s.pos:], data[pos:pos+take])
		s.pos += take
		pos += take
		if s.pos == 64 {
			s.process(s.buffer[:], 0)
			s.pos = 0
		}
	}
	s.length += uint64(n)
}

func (s *sha256) digest() []byte {
	pos := s.pos
	s.buffer[pos] = 0x80
	pos++
	clear(s.buffer[pos:])
	// No room for the 8-byte length field: pad this block and put the length in the next.
	if 8 > 64-pos {
		s.process(s.buffer[:], 0)
		clear(s.buffer[:])
	}
	bitLen := s.length * 8
	putUint32BE(s.buffer[:], 56, uint32(bitLen>>32))
	putUint32BE(s.buffer[:], 60, uint32(bitLen))
	s.process(s.buffer[:], 0)
	out := make([]byte, 32)
	for i, word := range [8]uint32{s.a, s.b, s.c, s.d, s.e, s.f, s.g, s.h} {
		putUint32BE(out, 4*i, word)
	}
	return out
}

// sha512 is noble's SHA2_64B with the SHA-512 IV: eight 64-bit words as high and low halves.
type sha512 struct {
	state  [16]uint32 // Ah, Al, Bh, Bl, ..., Hh, Hl
	wh, wl [80]uint32 // the message schedule's halves (noble's SHA512_W_H and SHA512_W_L)
	buffer [128]byte
	pos    int
	length uint64
}

func newSHA512() *sha512 { return &sha512{state: sha512IV} }

func (s *sha512) blockLen() int  { return 128 }
func (s *sha512) outputLen() int { return 64 }

func (s *sha512) process(data []byte, offset int) {
	wh, wl := &s.wh, &s.wl
	for i := 0; i < 16; i++ {
		wh[i] = getUint32BE(data, offset+8*i)
		wl[i] = getUint32BE(data, offset+8*i+4)
	}
	for i := 16; i < 80; i++ {
		// s0 = rotr(w[i-15], 1) ^ rotr(w[i-15], 8) ^ (w[i-15] >>> 7)
		w15h, w15l := wh[i-15], wl[i-15]
		s0h := rotrSH(w15h, w15l, 1) ^ rotrSH(w15h, w15l, 8) ^ shrSH(w15h, w15l, 7)
		s0l := rotrSL(w15h, w15l, 1) ^ rotrSL(w15h, w15l, 8) ^ shrSL(w15h, w15l, 7)
		// s1 = rotr(w[i-2], 19) ^ rotr(w[i-2], 61) ^ (w[i-2] >>> 6)
		w2h, w2l := wh[i-2], wl[i-2]
		s1h := rotrSH(w2h, w2l, 19) ^ rotrBH(w2h, w2l, 61) ^ shrSH(w2h, w2l, 6)
		s1l := rotrSL(w2h, w2l, 19) ^ rotrBL(w2h, w2l, 61) ^ shrSL(w2h, w2l, 6)
		wh[i], wl[i] = add4(s0h, s0l, s1h, s1l, wh[i-7], wl[i-7], wh[i-16], wl[i-16])
	}
	st := &s.state
	ah, al, bh, bl, ch, cl, dh, dl := st[0], st[1], st[2], st[3], st[4], st[5], st[6], st[7]
	eh, el, fh, fl, gh, gl, hh, hl := st[8], st[9], st[10], st[11], st[12], st[13], st[14], st[15]
	for i := 0; i < 80; i++ {
		// S1 = rotr(e, 14) ^ rotr(e, 18) ^ rotr(e, 41)
		sigma1h := rotrSH(eh, el, 14) ^ rotrSH(eh, el, 18) ^ rotrBH(eh, el, 41)
		sigma1l := rotrSL(eh, el, 14) ^ rotrSL(eh, el, 18) ^ rotrBL(eh, el, 41)
		// T1 = H + S1 + Chi(E, F, G) + K[i] + W[i]
		t1h, t1l := add5(hh, hl, sigma1h, sigma1l, chi(eh, fh, gh), chi(el, fl, gl), sha512Kh[i], sha512Kl[i], wh[i], wl[i])
		// S0 = rotr(a, 28) ^ rotr(a, 34) ^ rotr(a, 39)
		sigma0h := rotrSH(ah, al, 28) ^ rotrBH(ah, al, 34) ^ rotrBH(ah, al, 39)
		sigma0l := rotrSL(ah, al, 28) ^ rotrBL(ah, al, 34) ^ rotrBL(ah, al, 39)
		majh, majl := maj(ah, bh, ch), maj(al, bl, cl)
		hh, hl = gh, gl
		gh, gl = fh, fl
		fh, fl = eh, el
		eh, el = add(dh, dl, t1h, t1l)
		dh, dl = ch, cl
		ch, cl = bh, bl
		bh, bl = ah, al
		ah, al = add3(t1h, t1l, sigma0h, sigma0l, majh, majl)
	}
	// Add the compressed block to the state.
	working := [16]uint32{ah, al, bh, bl, ch, cl, dh, dl, eh, el, fh, fl, gh, gl, hh, hl}
	for i := 0; i < 16; i += 2 {
		st[i], st[i+1] = add(st[i], st[i+1], working[i], working[i+1])
	}
}

func (s *sha512) update(data []byte) {
	n := len(data)
	for pos := 0; pos < n; {
		take := min(128-s.pos, n-pos)
		// With nothing buffered, whole blocks are compressed straight from the input.
		if take == 128 {
			for ; 128 <= n-pos; pos += 128 {
				s.process(data, pos)
			}
			continue
		}
		copy(s.buffer[s.pos:], data[pos:pos+take])
		s.pos += take
		pos += take
		if s.pos == 128 {
			s.process(s.buffer[:], 0)
			s.pos = 0
		}
	}
	s.length += uint64(n)
}

func (s *sha512) digest() []byte {
	pos := s.pos
	s.buffer[pos] = 0x80
	pos++
	clear(s.buffer[pos:])
	// No room for the 16-byte length field: pad this block and put the length in the next.
	if 16 > 128-pos {
		s.process(s.buffer[:], 0)
		clear(s.buffer[:])
	}
	// Like noble, only the low 64 bits of the length field; the rest stays zero.
	bitLen := s.length * 8
	putUint32BE(s.buffer[:], 120, uint32(bitLen>>32))
	putUint32BE(s.buffer[:], 124, uint32(bitLen))
	s.process(s.buffer[:], 0)
	out := make([]byte, 64)
	for i, word := range s.state {
		putUint32BE(out, 4*i, word)
	}
	return out
}
