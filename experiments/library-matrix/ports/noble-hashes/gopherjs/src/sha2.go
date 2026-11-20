// SHA-256 and SHA-512 compression and streaming HMAC, adapted from @noble/hashes 2.4.0.
package main

import "math/bits"

var shaIV256 = [...]uint32{
	0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c,
	0x1f83d9ab, 0x5be0cd19,
}

var shaIV512 = [...]uint32{
	0x6a09e667, 0xf3bcc908, 0xbb67ae85, 0x84caa73b, 0x3c6ef372, 0xfe94f82b,
	0xa54ff53a, 0x5f1d36f1, 0x510e527f, 0xade682d1, 0x9b05688c, 0x2b3e6c1f,
	0x1f83d9ab, 0xfb41bd6b, 0x5be0cd19, 0x137e2179,
}

var shaK256 = [...]uint32{
	0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1,
	0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
	0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
	0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
	0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147,
	0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
	0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
	0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
	0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
	0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
	0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
}

var shaK512h = [...]uint32{
	0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1,
	0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
	0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
	0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
	0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147,
	0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
	0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
	0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
	0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
	0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
	0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2, 0xca273ece, 0xd186b8c7,
	0xeada7dd6, 0xf57d4f7f, 0x06f067aa, 0x0a637dc5, 0x113f9804, 0x1b710b35,
	0x28db77f5, 0x32caab7b, 0x3c9ebe0a, 0x431d67c4, 0x4cc5d4be, 0x597f299c,
	0x5fcb6fab, 0x6c44198c,
}

var shaK512l = [...]uint32{
	0xd728ae22, 0x23ef65cd, 0xec4d3b2f, 0x8189dbbc, 0xf348b538, 0xb605d019,
	0xaf194f9b, 0xda6d8118, 0xa3030242, 0x45706fbe, 0x4ee4b28c, 0xd5ffb4e2,
	0xf27b896f, 0x3b1696b1, 0x25c71235, 0xcf692694, 0x9ef14ad2, 0x384f25e3,
	0x8b8cd5b5, 0x77ac9c65, 0x592b0275, 0x6ea6e483, 0xbd41fbd4, 0x831153b5,
	0xee66dfab, 0x2db43210, 0x98fb213f, 0xbeef0ee4, 0x3da88fc2, 0x930aa725,
	0xe003826f, 0x0a0e6e70, 0x46d22ffc, 0x5c26c926, 0x5ac42aed, 0x9d95b3df,
	0x8baf63de, 0x3c77b2a8, 0x47edaee6, 0x1482353b, 0x4cf10364, 0xbc423001,
	0xd0f89791, 0x0654be30, 0xd6ef5218, 0x5565a910, 0x5771202a, 0x32bbd1b8,
	0xb8d2d0c8, 0x5141ab53, 0xdf8eeb99, 0xe19b48a8, 0xc5c95a63, 0xe3418acb,
	0x7763e373, 0xd6b2b8a3, 0x5defb2fc, 0x43172f60, 0xa1f0ab72, 0x1a6439ec,
	0x23631e28, 0xde82bde9, 0xb2c67915, 0xe372532b, 0xea26619c, 0x21c0c207,
	0xcde0eb1e, 0xee6ed178, 0x72176fba, 0xa2c898a6, 0xbef90dae, 0x131c471b,
	0x23047d84, 0x40c72493, 0x15c9bebc, 0x9c100d4c, 0xcb3e42b6, 0xfc657e2a,
	0x3ad6faec, 0x4a475817,
}

func shaRor(x uint32, n int) uint32 {
	return bits.RotateLeft32(x, -n)
}
func shaChoice(a uint32, b uint32, c uint32) uint32 {
	return ((a & b) ^ (^a & c))
}
func shaMajority(a uint32, b uint32, c uint32) uint32 {
	return (((a & b) ^ (a & c)) ^ (b & c))
}
func shaCarry(a uint32, b uint32) uint32 {
	if a+b < a {
		return 1
	}
	return 0
}
func shaRead(data []byte, at int) uint32 {
	return ((((uint32(data[at]) << 24) | (uint32(data[at+1]) << 16)) | (uint32(data[at+2]) << 8)) | uint32(data[at+3]))
}

func shaCompress256(state []uint32, data []byte, at int, wh []uint32) {
	for i := 0; i < 16; i++ {
		wh[i] = shaRead(data, at+i*4)
	}
	for i := 16; i < 64; i++ {
		x := wh[i-15]
		y := wh[i-2]
		s0 := ((shaRor(x, 7) ^ shaRor(x, 18)) ^ (x >> 3))
		s1 := ((shaRor(y, 17) ^ shaRor(y, 19)) ^ (y >> 10))
		wh[i] = (s1 + wh[i-7] + s0 + wh[i-16])
	}
	a := state[0]
	b := state[1]
	c := state[2]
	d := state[3]
	e := state[4]
	f := state[5]
	g := state[6]
	h := state[7]
	for i := 0; i < 64; i++ {
		s1 := ((shaRor(e, 6) ^ shaRor(e, 11)) ^ shaRor(e, 25))
		t1 := (h + s1 + shaChoice(e, f, g) + shaK256[i] + wh[i])
		s0 := ((shaRor(a, 2) ^ shaRor(a, 13)) ^ shaRor(a, 22))
		newA := (t1 + s0 + shaMajority(a, b, c))
		newE := (d + t1)
		h = g
		g = f
		f = e
		e = newE
		d = c
		c = b
		b = a
		a = newA
	}
	state[0] = (state[0] + a)
	state[1] = (state[1] + b)
	state[2] = (state[2] + c)
	state[3] = (state[3] + d)
	state[4] = (state[4] + e)
	state[5] = (state[5] + f)
	state[6] = (state[6] + g)
	state[7] = (state[7] + h)
}

func shaCompress512(state []uint32, data []byte, at int, wh []uint32, wl []uint32) {
	for i := 0; i < 16; i++ {
		wh[i] = shaRead(data, at+i*8)
		wl[i] = shaRead(data, at+i*8+4)
	}
	for i := 16; i < 80; i++ {
		xh := wh[i-15]
		xl := wl[i-15]
		yh := wh[i-2]
		yl := wl[i-2]
		s0h := ((((xh >> 1) | (xl << 31)) ^ ((xh >> 8) | (xl << 24))) ^ (xh >> 7))
		s0l := ((((xl >> 1) | (xh << 31)) ^ ((xl >> 8) | (xh << 24))) ^ ((xl >> 7) | (xh << 25)))
		s1h := ((((yh >> 19) | (yl << 13)) ^ ((yl >> 29) | (yh << 3))) ^ (yh >> 6))
		s1l := ((((yl >> 19) | (yh << 13)) ^ ((yh >> 29) | (yl << 3))) ^ ((yl >> 6) | (yh << 26)))
		nextl2 := (s0l + s1l)
		nextl3 := (nextl2 + wl[i-7])
		nextl := (nextl3 + wl[i-16])
		nexth := (s0h + s1h + wh[i-7] + wh[i-16] + shaCarry(s0l, s1l) + shaCarry(nextl2, wl[i-7]) + shaCarry(nextl3, wl[i-16]))
		wh[i] = nexth
		wl[i] = nextl
	}
	ah := state[0]
	al := state[1]
	bh := state[2]
	bl := state[3]
	ch := state[4]
	cl := state[5]
	dh := state[6]
	dl := state[7]
	eh := state[8]
	el := state[9]
	fh := state[10]
	fl := state[11]
	gh := state[12]
	gl := state[13]
	hh := state[14]
	hl := state[15]
	for i := 0; i < 80; i++ {
		s1h := ((((eh >> 14) | (el << 18)) ^ ((eh >> 18) | (el << 14))) ^ ((el >> 9) | (eh << 23)))
		s1l := ((((el >> 14) | (eh << 18)) ^ ((el >> 18) | (eh << 14))) ^ ((eh >> 9) | (el << 23)))
		chh := shaChoice(eh, fh, gh)
		chl := shaChoice(el, fl, gl)
		t1l2 := (hl + s1l)
		t1l3 := (t1l2 + chl)
		t1l4 := (t1l3 + shaK512l[i])
		t1l := (t1l4 + wl[i])
		t1h := (hh + s1h + chh + shaK512h[i] + wh[i] + shaCarry(hl, s1l) + shaCarry(t1l2, chl) + shaCarry(t1l3, shaK512l[i]) + shaCarry(t1l4, wl[i]))
		s0h := ((((ah >> 28) | (al << 4)) ^ ((al >> 2) | (ah << 30))) ^ ((al >> 7) | (ah << 25)))
		s0l := ((((al >> 28) | (ah << 4)) ^ ((ah >> 2) | (al << 30))) ^ ((ah >> 7) | (al << 25)))
		majh := shaMajority(ah, bh, ch)
		majl := shaMajority(al, bl, cl)
		newAl2 := (t1l + s0l)
		newAl := (newAl2 + majl)
		newAh := (t1h + s0h + majh + shaCarry(t1l, s0l) + shaCarry(newAl2, majl))
		newEl := (dl + t1l)
		newEh := (dh + t1h + shaCarry(dl, t1l))
		hh = gh
		hl = gl
		gh = fh
		gl = fl
		fh = eh
		fl = el
		eh = newEh
		el = newEl
		dh = ch
		dl = cl
		ch = bh
		cl = bl
		bh = ah
		bl = al
		ah = newAh
		al = newAl
	}
	state[0] = (state[0] + ah + shaCarry(state[1], al))
	state[1] = (state[1] + al)
	state[2] = (state[2] + bh + shaCarry(state[3], bl))
	state[3] = (state[3] + bl)
	state[4] = (state[4] + ch + shaCarry(state[5], cl))
	state[5] = (state[5] + cl)
	state[6] = (state[6] + dh + shaCarry(state[7], dl))
	state[7] = (state[7] + dl)
	state[8] = (state[8] + eh + shaCarry(state[9], el))
	state[9] = (state[9] + el)
	state[10] = (state[10] + fh + shaCarry(state[11], fl))
	state[11] = (state[11] + fl)
	state[12] = (state[12] + gh + shaCarry(state[13], gl))
	state[13] = (state[13] + gl)
	state[14] = (state[14] + hh + shaCarry(state[15], hl))
	state[15] = (state[15] + hl)
}

func shaHash(wide bool, chunks [][]byte) []byte {
	block := 64
	state := append([]uint32(nil), shaIV256[:]...)
	if wide {
		block = 128
		state = append([]uint32(nil), shaIV512[:]...)
	}
	wh := make([]uint32, 80)
	wl := make([]uint32, 80)
	tail := make([]byte, block)
	process := func(chunk []byte, at int) {
		if wide {
			shaCompress512(state, chunk, at, wh, wl)
		} else {
			shaCompress256(state, chunk, at, wh)
		}
	}
	var length uint64
	pos := 0
	for _, chunk := range chunks {
		length += uint64(len(chunk))
		for at := 0; at < len(chunk); {
			if pos == 0 && len(chunk)-at >= block {
				process(chunk, at)
				at += block
				continue
			}
			n := copy(tail[pos:], chunk[at:])
			pos += n
			at += n
			if pos == block {
				process(tail, 0)
				pos = 0
			}
		}
	}
	tail[pos] = 0x80
	for i := pos + 1; i < block; i++ {
		tail[i] = 0
	}
	if pos+1 > block-block/8 {
		process(tail, 0)
		for i := range tail {
			tail[i] = 0
		}
	}
	bitsLen := length * 8
	for i := 0; i < 8; i++ {
		tail[block-1-i] = byte(bitsLen)
		bitsLen >>= 8
	}
	process(tail, 0)
	out := make([]byte, len(state)*4)
	for i, w := range state {
		for j := 0; j < 4; j++ {
			out[i*4+j] = byte(w >> uint(24-j*8))
		}
	}
	return out
}

func sha2Digest(mode int, chunks [][]byte, key []byte) []byte {
	if mode != 2 {
		return shaHash(mode == 1, chunks)
	}
	if len(key) > 64 {
		key = shaHash(false, [][]byte{key})
	}
	inner, outer := make([]byte, 64), make([]byte, 64)
	for i := range inner {
		var b byte
		if i < len(key) {
			b = key[i]
		}
		inner[i] = b ^ 0x36
		outer[i] = b ^ 0x5c
	}
	mac := shaHash(false, append([][]byte{inner}, chunks...))
	return shaHash(false, [][]byte{outer, mac})
}
