package main

// noble's sha3.ts: Keccak-f[1600] and the Keccak sponge behind SHA3-256 and Keccak-256.
//
// The state is noble's state32: 25 lanes of two 32-bit words, the low word first. noble derives
// the pi, rho and iota tables at load time; they are written out here.

var (
	// sha3Pi is the word index each rho step moves its lane to (noble's SHA3_PI).
	sha3Pi = [24]int{20, 14, 22, 34, 36, 6, 10, 32, 16, 42, 48, 8, 30, 46, 38, 26, 24, 4, 40, 28, 44, 18, 12, 2}

	// sha3Rotl is the left rotation of each step (noble's SHA3_ROTL).
	sha3Rotl = [24]uint{1, 3, 6, 10, 15, 21, 28, 36, 45, 55, 2, 14, 27, 41, 56, 8, 25, 43, 62, 18, 39, 61, 20, 44}

	// sha3IotaH and sha3IotaL are the low and high words of each round constant.
	sha3IotaH = [24]uint32{
		0x00000001, 0x00008082, 0x0000808a, 0x80008000, 0x0000808b, 0x80000001, 0x80008081, 0x00008009,
		0x0000008a, 0x00000088, 0x80008009, 0x8000000a, 0x8000808b, 0x0000008b, 0x00008089, 0x00008003,
		0x00008002, 0x00000080, 0x0000800a, 0x8000000a, 0x80008081, 0x00008080, 0x80000001, 0x80008008,
	}
	sha3IotaL = [24]uint32{
		0x00000000, 0x00000000, 0x80000000, 0x80000000, 0x00000000, 0x00000000, 0x80000000, 0x80000000,
		0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x80000000, 0x80000000, 0x80000000,
		0x80000000, 0x80000000, 0x00000000, 0x80000000, 0x80000000, 0x80000000, 0x00000000, 0x80000000,
	}
)

// Left rotations of a lane by s in 1..31 (S) or 33..63 (B). As in noble, h is the lane's first
// (low) word and l its second (high) word.
func rotlSH(h, l uint32, s uint) uint32 { return h<<s | l>>(32-s) }
func rotlSL(h, l uint32, s uint) uint32 { return l<<s | h>>(32-s) }
func rotlBH(h, l uint32, s uint) uint32 { return l<<(s-32) | h>>(64-s) }
func rotlBL(h, l uint32, s uint) uint32 { return h<<(s-32) | l>>(64-s) }

func rotlH(h, l uint32, s uint) uint32 {
	if s > 32 {
		return rotlBH(h, l, s)
	}
	return rotlSH(h, l, s)
}

func rotlL(h, l uint32, s uint) uint32 {
	if s > 32 {
		return rotlBL(h, l, s)
	}
	return rotlSL(h, l, s)
}

// keccakP is noble's: the 24 rounds of Keccak-f[1600] on s, with b as the theta scratch.
func keccakP(s *[50]uint32, b *[10]uint32) {
	for round := 0; round < 24; round++ {
		// Theta
		for x := 0; x < 10; x++ {
			b[x] = s[x] ^ s[x+10] ^ s[x+20] ^ s[x+30] ^ s[x+40]
		}
		for x := 0; x < 10; x += 2 {
			idx1 := (x + 8) % 10
			idx0 := (x + 2) % 10
			b0, b1 := b[idx0], b[idx0+1]
			th := rotlH(b0, b1, 1) ^ b[idx1]
			tl := rotlL(b0, b1, 1) ^ b[idx1+1]
			for y := 0; y < 50; y += 10 {
				s[x+y] ^= th
				s[x+y+1] ^= tl
			}
		}
		// Rho and pi
		curH, curL := s[2], s[3]
		for t := 0; t < 24; t++ {
			shift := sha3Rotl[t]
			th, tl := rotlH(curH, curL, shift), rotlL(curH, curL, shift)
			pi := sha3Pi[t]
			curH, curL = s[pi], s[pi+1]
			s[pi], s[pi+1] = th, tl
		}
		// Chi
		for y := 0; y < 50; y += 10 {
			b0, b1, b2, b3 := s[y], s[y+1], s[y+2], s[y+3]
			for x := 0; x < 6; x++ {
				s[y+x] ^= ^s[y+x+2] & s[y+x+4]
			}
			s[y+6] ^= ^s[y+8] & b0
			s[y+7] ^= ^s[y+9] & b1
			s[y+8] ^= ^b0 & b2
			s[y+9] ^= ^b1 & b3
		}
		// Iota
		s[0] ^= sha3IotaH[round]
		s[1] ^= sha3IotaL[round]
	}
}

// keccak is noble's Keccak sponge with a byte-length rate and a domain suffix.
type keccak struct {
	rate, suffix, size int
	state32            [50]uint32
	theta              [10]uint32
	pos, posOut        int
}

func newKeccak(blockLen, suffix, outputLen int) *keccak {
	return &keccak{rate: blockLen, suffix: suffix, size: outputLen}
}

func (k *keccak) blockLen() int  { return k.rate }
func (k *keccak) outputLen() int { return k.size }

// xorByte XORs v into state byte i, as noble does through its byte view of state32.
func (k *keccak) xorByte(i int, v byte) { k.state32[i>>2] ^= uint32(v) << (8 * (i & 3)) }

// stateByte returns state byte i.
func (k *keccak) stateByte(i int) byte { return byte(k.state32[i>>2] >> (8 * (i & 3))) }

func (k *keccak) keccak() {
	keccakP(&k.state32, &k.theta)
	k.posOut = 0
	k.pos = 0
}

func (k *keccak) update(data []byte) {
	n := len(data)
	for pos := 0; pos < n; {
		// Whole blocks with nothing buffered are absorbed a word at a time.
		if k.pos == 0 && n-pos >= k.rate {
			for i := 0; i < k.rate/4; i++ {
				k.state32[i] ^= getUint32LE(data, pos+4*i)
			}
			pos += k.rate
			k.pos = k.rate
			k.keccak()
			continue
		}
		take := min(k.rate-k.pos, n-pos)
		for i := 0; i < take; i++ {
			k.xorByte(k.pos, data[pos])
			k.pos++
			pos++
		}
		if k.pos == k.rate {
			k.keccak()
		}
	}
}

func (k *keccak) finish() {
	// The suffix holds the domain bits and the first padding bit; 0x80 closes pad10*1.
	k.xorByte(k.pos, byte(k.suffix))
	if k.suffix&0x80 != 0 && k.pos == k.rate-1 {
		k.keccak()
	}
	k.xorByte(k.rate-1, 0x80)
	k.keccak()
}

func (k *keccak) digest() []byte {
	k.finish()
	out := make([]byte, k.size)
	for pos := 0; pos < k.size; {
		if k.posOut >= k.rate {
			k.keccak()
		}
		take := min(k.rate-k.posOut, k.size-pos)
		for i := 0; i < take; i++ {
			out[pos] = k.stateByte(k.posOut)
			k.posOut++
			pos++
		}
	}
	return out
}
