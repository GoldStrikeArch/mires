package main

// noble's blake2.ts and _blake.ts: BLAKE2b and BLAKE2s.
//
// BLAKE2b keeps noble's G1b/G2b on 64-bit words as low and high halves in a 32-word work vector
// (noble's BBUF); BLAKE2s keeps G1s/G2s, which return the four mixed words as noble's do. The two
// do not share a mix function. Each carries noble's _BLAKE2 update and digest, for the reason
// sha2.go gives.

var (
	// bsigma is the message word order of each round (noble's BSIGMA); BLAKE2b's rounds 10 and
	// 11 repeat 0 and 1.
	bsigma = [12 * 16]uint8{
		0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
		14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3,
		11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4,
		7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8,
		9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13,
		2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9,
		12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11,
		13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10,
		6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5,
		10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0,
		0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
		14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3,
	}

	// b2bIV is the SHA-512 IV as little-endian low and high halves (noble's B2B_IV).
	b2bIV = [16]uint32{
		0xf3bcc908, 0x6a09e667, 0x84caa73b, 0xbb67ae85, 0xfe94f82b, 0x3c6ef372, 0x5f1d36f1, 0xa54ff53a,
		0xade682d1, 0x510e527f, 0x2b3e6c1f, 0x9b05688c, 0xfb41bd6b, 0x1f83d9ab, 0x137e2179, 0x5be0cd19,
	}
)

// blake2Opts is noble's Blake2Opts: the options reference.mjs passes to every hash. A nil
// key, salt or personalization is absent.
type blake2Opts struct {
	dkLen                      *float64
	key, salt, personalization []byte
}

// checkBlake2Opts is noble's; keyLen bounds both the digest and the key length.
func checkBlake2Opts(outputLen int, opts blake2Opts, keyLen, saltLen, persLen int) error {
	switch {
	case outputLen <= 0 || outputLen > keyLen:
		return hashError(`"dkLen" out of range`)
	case opts.key != nil && (len(opts.key) < 1 || len(opts.key) > keyLen):
		return hashError(`"key" expected to be undefined or of a valid length`)
	case opts.salt != nil && len(opts.salt) != saltLen:
		return hashError(`"salt" has the wrong length`)
	case opts.personalization != nil && len(opts.personalization) != persLen:
		return hashError(`"personalization" has the wrong length`)
	}
	return nil
}

// outputLength is the digest length opts asks for, def when absent, checked by anumber.
func outputLength(opts blake2Opts, def int) (int, error) {
	if opts.dkLen == nil {
		return def, nil
	}
	n, err := anumber(*opts.dkLen)
	if err != nil {
		return 0, hashError(`"outputLen" ` + err.Error())
	}
	return n, nil
}

// blake2b is noble's _BLAKE2b.
type blake2b struct {
	v      [16]uint32 // v0l, v0h, ..., v7l, v7h
	work   [32]uint32 // the work vector of 16 words as low and high halves (noble's BBUF)
	msg    [32]uint32 // the block being compressed as little-endian 32-bit words
	buffer [128]byte
	pos    int
	length uint64 // bytes compressed or buffered so far: the BLAKE2 offset counter
	size   int
}

func newBlake2b(opts blake2Opts) (*blake2b, error) {
	size, err := outputLength(opts, 64)
	if err != nil {
		return nil, err
	}
	if err := checkBlake2Opts(size, opts, 64, 16, 16); err != nil {
		return nil, err
	}
	s := &blake2b{v: b2bIV, size: size}
	// RFC 7693 §2.5: p[0] = 0x0101kknn goes into the low half of h[0].
	s.v[0] ^= uint32(size) | uint32(len(opts.key))<<8 | 0x01<<16 | 0x01<<24
	if opts.salt != nil {
		for i := 0; i < 4; i++ {
			s.v[8+i] ^= getUint32LE(opts.salt, 4*i)
		}
	}
	if opts.personalization != nil {
		for i := 0; i < 4; i++ {
			s.v[12+i] ^= getUint32LE(opts.personalization, 4*i)
		}
	}
	if opts.key != nil {
		// The zero-padded key is the first block.
		var block [128]byte
		copy(block[:], opts.key)
		s.update(block[:])
	}
	return s, nil
}

func (s *blake2b) blockLen() int  { return 128 }
func (s *blake2b) outputLen() int { return s.size }

// g1b is G's first half: a = a + b + x, d = rotr(d ^ a, 32), c = c + d, b = rotr(b ^ c, 24).
func (s *blake2b) g1b(a, b, c, d, x int) {
	v := &s.work
	ah, al := add3(v[2*a+1], v[2*a], v[2*b+1], v[2*b], s.msg[x+1], s.msg[x])
	th, tl := v[2*d+1]^ah, v[2*d]^al
	dh, dl := rotr32H(th, tl), rotr32L(th, tl)
	ch, cl := add(v[2*c+1], v[2*c], dh, dl)
	th, tl = v[2*b+1]^ch, v[2*b]^cl
	v[2*a], v[2*a+1] = al, ah
	v[2*b], v[2*b+1] = rotrSL(th, tl, 24), rotrSH(th, tl, 24)
	v[2*c], v[2*c+1] = cl, ch
	v[2*d], v[2*d+1] = dl, dh
}

// g2b is G's second half: a = a + b + y, d = rotr(d ^ a, 16), c = c + d, b = rotr(b ^ c, 63).
func (s *blake2b) g2b(a, b, c, d, x int) {
	v := &s.work
	ah, al := add3(v[2*a+1], v[2*a], v[2*b+1], v[2*b], s.msg[x+1], s.msg[x])
	th, tl := v[2*d+1]^ah, v[2*d]^al
	dh, dl := rotrSH(th, tl, 16), rotrSL(th, tl, 16)
	ch, cl := add(v[2*c+1], v[2*c], dh, dl)
	th, tl = v[2*b+1]^ch, v[2*b]^cl
	v[2*a], v[2*a+1] = al, ah
	v[2*b], v[2*b+1] = rotrBL(th, tl, 63), rotrBH(th, tl, 63)
	v[2*c], v[2*c+1] = cl, ch
	v[2*d], v[2*d+1] = dl, dh
}

func (s *blake2b) compress(data []byte, offset int, isLast bool) {
	for i := range s.msg {
		s.msg[i] = getUint32LE(data, offset+4*i)
	}
	v := &s.work
	copy(v[:16], s.v[:])
	copy(v[16:], b2bIV[:])
	// The byte counter goes into v12; v14 is inverted on the last block.
	v[24] = b2bIV[8] ^ uint32(s.length)
	v[25] = b2bIV[9] ^ uint32(s.length>>32)
	if isLast {
		v[28], v[29] = ^v[28], ^v[29]
	}
	// bsigma picks 64-bit message words; each spans two entries of msg.
	for r := 0; r < 12; r++ {
		sigma := bsigma[16*r : 16*r+16]
		s.g1b(0, 4, 8, 12, 2*int(sigma[0]))
		s.g2b(0, 4, 8, 12, 2*int(sigma[1]))
		s.g1b(1, 5, 9, 13, 2*int(sigma[2]))
		s.g2b(1, 5, 9, 13, 2*int(sigma[3]))
		s.g1b(2, 6, 10, 14, 2*int(sigma[4]))
		s.g2b(2, 6, 10, 14, 2*int(sigma[5]))
		s.g1b(3, 7, 11, 15, 2*int(sigma[6]))
		s.g2b(3, 7, 11, 15, 2*int(sigma[7]))
		s.g1b(0, 5, 10, 15, 2*int(sigma[8]))
		s.g2b(0, 5, 10, 15, 2*int(sigma[9]))
		s.g1b(1, 6, 11, 12, 2*int(sigma[10]))
		s.g2b(1, 6, 11, 12, 2*int(sigma[11]))
		s.g1b(2, 7, 8, 13, 2*int(sigma[12]))
		s.g2b(2, 7, 8, 13, 2*int(sigma[13]))
		s.g1b(3, 4, 9, 14, 2*int(sigma[14]))
		s.g2b(3, 4, 9, 14, 2*int(sigma[15]))
	}
	for i := range s.v {
		s.v[i] ^= v[i] ^ v[i+16]
	}
}

// update withholds the last block: it is compressed with a flag, so a full buffer waits for
// more input.
func (s *blake2b) update(data []byte) {
	n := len(data)
	for pos := 0; pos < n; {
		if s.pos == 128 {
			s.compress(s.buffer[:], 0, false)
			s.pos = 0
		}
		take := min(128-s.pos, n-pos)
		// Whole blocks that are not the last of the input are compressed straight from it.
		if take == 128 && pos+take < n {
			for ; pos+128 < n; pos += 128 {
				s.length += 128
				s.compress(data, pos, false)
			}
			continue
		}
		copy(s.buffer[s.pos:], data[pos:pos+take])
		s.pos += take
		s.length += uint64(take)
		pos += take
	}
}

func (s *blake2b) digest() []byte {
	clear(s.buffer[s.pos:])
	s.compress(s.buffer[:], 0, true)
	return wordsToBytesLE(s.v[:], s.size)
}

// wordsToBytesLE returns the first n bytes of the little-endian encoding of words.
func wordsToBytesLE(words []uint32, n int) []byte {
	out := make([]byte, n)
	for i := range out {
		out[i] = byte(words[i/4] >> (8 * (i % 4)))
	}
	return out
}

// g1s is BLAKE2s's G first half: a = a + b + x, d = rotr(d ^ a, 16), c = c + d, b = rotr(b ^ c, 12).
func g1s(a, b, c, d, x uint32) (uint32, uint32, uint32, uint32) {
	a += b + x
	d = rotr(d^a, 16)
	c += d
	b = rotr(b^c, 12)
	return a, b, c, d
}

// g2s is BLAKE2s's G second half: a = a + b + y, d = rotr(d ^ a, 8), c = c + d, b = rotr(b ^ c, 7).
func g2s(a, b, c, d, x uint32) (uint32, uint32, uint32, uint32) {
	a += b + x
	d = rotr(d^a, 8)
	c += d
	b = rotr(b^c, 7)
	return a, b, c, d
}

// blake2s is noble's _BLAKE2s, whose IV is SHA-256's.
type blake2s struct {
	v      [8]uint32
	msg    [16]uint32 // the block being compressed as little-endian 32-bit words
	buffer [64]byte
	pos    int
	length uint64 // bytes compressed or buffered so far: the BLAKE2 offset counter
	size   int
}

func newBlake2s(opts blake2Opts) (*blake2s, error) {
	size, err := outputLength(opts, 32)
	if err != nil {
		return nil, err
	}
	if err := checkBlake2Opts(size, opts, 32, 8, 8); err != nil {
		return nil, err
	}
	s := &blake2s{v: sha256IV, size: size}
	s.v[0] ^= uint32(size) | uint32(len(opts.key))<<8 | 0x01<<16 | 0x01<<24
	if opts.salt != nil {
		s.v[4] ^= getUint32LE(opts.salt, 0)
		s.v[5] ^= getUint32LE(opts.salt, 4)
	}
	if opts.personalization != nil {
		s.v[6] ^= getUint32LE(opts.personalization, 0)
		s.v[7] ^= getUint32LE(opts.personalization, 4)
	}
	if opts.key != nil {
		var block [64]byte
		copy(block[:], opts.key)
		s.update(block[:])
	}
	return s, nil
}

func (s *blake2s) blockLen() int  { return 64 }
func (s *blake2s) outputLen() int { return s.size }

func (s *blake2s) compress(data []byte, offset int, isLast bool) {
	m := &s.msg
	for i := range m {
		m[i] = getUint32LE(data, offset+4*i)
	}
	// noble's _compress: v8..v15 come from the IV, with the byte counter in v12 and v13 and v14
	// inverted on the last block.
	iv := sha256IV
	v0, v1, v2, v3, v4, v5, v6, v7 := s.v[0], s.v[1], s.v[2], s.v[3], s.v[4], s.v[5], s.v[6], s.v[7]
	v8, v9, v10, v11 := iv[0], iv[1], iv[2], iv[3]
	v12, v13, v14, v15 := uint32(s.length)^iv[4], uint32(s.length>>32)^iv[5], iv[6], iv[7]
	if isLast {
		v14 = ^v14
	}
	for r := 0; r < 10; r++ {
		sigma := bsigma[16*r : 16*r+16]
		v0, v4, v8, v12 = g1s(v0, v4, v8, v12, m[sigma[0]])
		v0, v4, v8, v12 = g2s(v0, v4, v8, v12, m[sigma[1]])
		v1, v5, v9, v13 = g1s(v1, v5, v9, v13, m[sigma[2]])
		v1, v5, v9, v13 = g2s(v1, v5, v9, v13, m[sigma[3]])
		v2, v6, v10, v14 = g1s(v2, v6, v10, v14, m[sigma[4]])
		v2, v6, v10, v14 = g2s(v2, v6, v10, v14, m[sigma[5]])
		v3, v7, v11, v15 = g1s(v3, v7, v11, v15, m[sigma[6]])
		v3, v7, v11, v15 = g2s(v3, v7, v11, v15, m[sigma[7]])
		v0, v5, v10, v15 = g1s(v0, v5, v10, v15, m[sigma[8]])
		v0, v5, v10, v15 = g2s(v0, v5, v10, v15, m[sigma[9]])
		v1, v6, v11, v12 = g1s(v1, v6, v11, v12, m[sigma[10]])
		v1, v6, v11, v12 = g2s(v1, v6, v11, v12, m[sigma[11]])
		v2, v7, v8, v13 = g1s(v2, v7, v8, v13, m[sigma[12]])
		v2, v7, v8, v13 = g2s(v2, v7, v8, v13, m[sigma[13]])
		v3, v4, v9, v14 = g1s(v3, v4, v9, v14, m[sigma[14]])
		v3, v4, v9, v14 = g2s(v3, v4, v9, v14, m[sigma[15]])
	}
	s.v[0] ^= v0 ^ v8
	s.v[1] ^= v1 ^ v9
	s.v[2] ^= v2 ^ v10
	s.v[3] ^= v3 ^ v11
	s.v[4] ^= v4 ^ v12
	s.v[5] ^= v5 ^ v13
	s.v[6] ^= v6 ^ v14
	s.v[7] ^= v7 ^ v15
}

// update withholds the last block, as blake2b's does.
func (s *blake2s) update(data []byte) {
	n := len(data)
	for pos := 0; pos < n; {
		if s.pos == 64 {
			s.compress(s.buffer[:], 0, false)
			s.pos = 0
		}
		take := min(64-s.pos, n-pos)
		if take == 64 && pos+take < n {
			for ; pos+64 < n; pos += 64 {
				s.length += 64
				s.compress(data, pos, false)
			}
			continue
		}
		copy(s.buffer[s.pos:], data[pos:pos+take])
		s.pos += take
		s.length += uint64(take)
		pos += take
	}
}

func (s *blake2s) digest() []byte {
	clear(s.buffer[s.pos:])
	s.compress(s.buffer[:], 0, true)
	return wordsToBytesLE(s.v[:], s.size)
}
