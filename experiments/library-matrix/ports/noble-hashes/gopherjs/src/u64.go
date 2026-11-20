package main

import "math/bits"

// noble's _u64.ts: a 64-bit word is a high and a low 32-bit half.
//
// Shifts and rotations return one half, as in noble. Sums return both halves; the carry out of
// the low halves is bits.Add32's, where noble divides its unmasked JS-number sum by 2^32. The 3-
// to 5-term sums are noble's add3L/add3H ... add5L/add5H, each computed in one function.

// shrSH is the high half of a 64-bit logical right shift by s in 0..31.
func shrSH(h, _ uint32, s uint) uint32 { return h >> s }

// shrSL is the low half of a 64-bit logical right shift by s in 1..31.
func shrSL(h, l uint32, s uint) uint32 { return h<<(32-s) | l>>s }

// rotrSH is the high half of a 64-bit right rotation by s in 1..31.
func rotrSH(h, l uint32, s uint) uint32 { return h>>s | l<<(32-s) }

// rotrSL is the low half of a 64-bit right rotation by s in 1..31.
func rotrSL(h, l uint32, s uint) uint32 { return h<<(32-s) | l>>s }

// rotrBH is the high half of a 64-bit right rotation by s in 33..63.
func rotrBH(h, l uint32, s uint) uint32 { return h<<(64-s) | l>>(s-32) }

// rotrBL is the low half of a 64-bit right rotation by s in 33..63.
func rotrBL(h, l uint32, s uint) uint32 { return h>>(s-32) | l<<(64-s) }

// rotr32H is the high half of a 64-bit right rotation by 32: the low half.
func rotr32H(_, l uint32) uint32 { return l }

// rotr32L is the low half of a 64-bit right rotation by 32: the high half.
func rotr32L(h, _ uint32) uint32 { return h }

// add returns the sum of two 64-bit words.
func add(ah, al, bh, bl uint32) (h, l uint32) {
	l, c := bits.Add32(al, bl, 0)
	return ah + bh + c, l
}

// add3 returns the sum of three 64-bit words.
func add3(ah, al, bh, bl, ch, cl uint32) (h, l uint32) {
	l, c1 := bits.Add32(al, bl, 0)
	l, c2 := bits.Add32(l, cl, 0)
	return ah + bh + ch + c1 + c2, l
}

// add4 returns the sum of four 64-bit words.
func add4(ah, al, bh, bl, ch, cl, dh, dl uint32) (h, l uint32) {
	l, c1 := bits.Add32(al, bl, 0)
	l, c2 := bits.Add32(l, cl, 0)
	l, c3 := bits.Add32(l, dl, 0)
	return ah + bh + ch + dh + c1 + c2 + c3, l
}

// add5 returns the sum of five 64-bit words.
func add5(ah, al, bh, bl, ch, cl, dh, dl, eh, el uint32) (h, l uint32) {
	l, c1 := bits.Add32(al, bl, 0)
	l, c2 := bits.Add32(l, cl, 0)
	l, c3 := bits.Add32(l, dl, 0)
	l, c4 := bits.Add32(l, el, 0)
	return ah + bh + ch + dh + eh + c1 + c2 + c3 + c4, l
}
