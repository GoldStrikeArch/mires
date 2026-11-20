// Canonical Huffman maps (hMap), Huffman tree construction and length clamping (hTree), RLE
// code-length encoding (lc), and bit-cost calculation (clen).
//
// Everything uses parallel primitive arrays and custom in-place stable insertion sort (no
// sort.SliceStable, no closures, no struct-element pointers) so GopherJS emits straight-line code
// with zero $restore state machines and zero $indexPtr overhead.
package main

// hMap builds either a forward encoding table (revMode == false) or a reverse decoding lookup
// table (revMode == true) from code lengths cd with maximum bit length mb (0..15).
func hMap(cd []uint8, mb int, revMode bool) []uint16 {
	s := len(cd)
	var l [16]uint16
	for i := 0; i < s; i++ {
		ci := cd[i]
		if ci != 0 {
			l[ci-1]++
		}
	}
	var le [16]uint16
	for i := 1; i < mb; i++ {
		le[i] = (le[i-1] + l[i-1]) << 1
	}
	if revMode {
		co := make([]uint16, 1<<uint(mb))
		rvb := uint(15 - mb)
		for i := 0; i < s; i++ {
			ci := int(cd[i])
			if ci != 0 {
				sv := uint16((i << 4) | ci)
				r1 := uint(mb - ci)
				code := int(le[ci-1])
				le[ci-1]++
				v := code << r1
				m := v | ((1 << r1) - 1)
				for ; v <= m; v++ {
					if v < 32768 {
						co[rev[v]>>rvb] = sv
					} else {
						co[0] = sv
					}
				}
			}
		}
		return co
	}
	co := make([]uint16, s)
	for i := 0; i < s; i++ {
		ci := int(cd[i])
		if ci != 0 {
			code := int(le[ci-1])
			le[ci-1]++
			if code < 32768 {
				co[i] = rev[code] >> uint(15-ci)
			}
		}
	}
	return co
}

// hTree computes canonical Huffman code lengths from symbol frequencies d, clamped to at most mb
// bits, preserving fflate 0.8.3 / UZIP.js tie-breaking bit-for-bit.
func hTree(d []uint16, mb int) ([]uint8, int) {
	// At most 288 symbols => at most 288 leaves + 1 sentinel + 287 internal nodes = 576 entries.
	var sym [577]int32
	var freq [577]int32
	var left [577]int32
	var right [577]int32
	var t2Sym [288]int32
	var t2Freq [288]int32

	s := 0
	for i := 0; i < len(d); i++ {
		if d[i] != 0 {
			sym[s] = int32(i)
			freq[s] = int32(d[i])
			t2Sym[s] = int32(i)
			t2Freq[s] = int32(d[i])
			s++
		}
	}
	if s == 0 {
		return make([]uint8, 0), 0
	}
	if s == 1 {
		v := make([]uint8, int(sym[0])+1)
		v[sym[0]] = 1
		return v, 1
	}

	// Stable insertion sort leaves 0..s-1 by ascending freq (ties preserve ascending sym).
	for a := 1; a < s; a++ {
		ks := sym[a]
		kf := freq[a]
		b := a - 1
		for b >= 0 && freq[b] > kf {
			sym[b+1] = sym[b]
			freq[b+1] = freq[b]
			b--
		}
		sym[b+1] = ks
		freq[b+1] = kf
	}

	// Sentinel at index s (greater than maximum possible block symbol sum 25000).
	sym[s] = -1
	freq[s] = 25001

	// Two-queue linear merge (UZIP.js). Internal nodes live at s+1 .. 2*s-1; strict `<` breaks
	// equal-frequency ties in favour of leaves (i2), matching fflate 0.8.3.
	i0 := s + 1
	i1 := s + 1
	i2 := 2
	sym[i1] = -1
	freq[i1] = freq[0] + freq[1]
	left[i1] = 0
	right[i1] = 1
	i1++
	end := 2 * s
	for i1 != end {
		var l, r int
		if freq[i0] < freq[i2] {
			l = i0
			i0++
		} else {
			l = i2
			i2++
		}
		if i0 != i1 && freq[i0] < freq[i2] {
			r = i0
			i0++
		} else {
			r = i2
			i2++
		}
		sym[i1] = -1
		freq[i1] = freq[l] + freq[r]
		left[i1] = int32(l)
		right[i1] = int32(r)
		i1++
	}

	// Because internal nodes are built bottom-up (children always have smaller indices than their
	// parent), walking backwards from root (2*s-1) down to s+1 assigns exact leaf depths iteratively.
	var depth [577]uint16
	for k := end - 1; k > s; k-- {
		d1 := depth[k] + 1
		depth[left[k]] = d1
		depth[right[k]] = d1
	}

	maxSym := int(t2Sym[s-1])
	tr := make([]uint16, maxSym+1)
	mbt := 0
	for k := 0; k < s; k++ {
		dk := depth[k]
		tr[sym[k]] = dk
		if int(dk) > mbt {
			mbt = int(dk)
		}
	}

	// Length clamping and signed Kraft debt redistribution + rollback when mbt > mb.
	if mbt > mb {
		// Stable insertion sort t2 by descending tr[s], then ascending f (ties keep ascending s).
		for a := 1; a < s; a++ {
			ks := t2Sym[a]
			kf := t2Freq[a]
			kt := tr[ks]
			b := a - 1
			for b >= 0 && (tr[t2Sym[b]] < kt || (tr[t2Sym[b]] == kt && t2Freq[b] > kf)) {
				t2Sym[b+1] = t2Sym[b]
				t2Freq[b+1] = t2Freq[b]
				b--
			}
			t2Sym[b+1] = ks
			t2Freq[b+1] = kf
		}
		i := 0
		dt := int32(0)
		lft := uint(mbt - mb)
		cst := int32(1) << lft
		for ; i < s; i++ {
			symI := t2Sym[i]
			if int(tr[symI]) > mb {
				dt += cst - (int32(1) << uint(mbt-int(tr[symI])))
				tr[symI] = uint16(mb)
			} else {
				break
			}
		}
		dt >>= lft
		for dt > 0 {
			symI := t2Sym[i]
			if int(tr[symI]) < mb {
				dt -= int32(1) << uint(mb-int(tr[symI])-1)
				tr[symI]++
			} else {
				i++
			}
		}
		for ; i >= 0 && dt != 0; i-- {
			symI := t2Sym[i]
			if int(tr[symI]) == mb {
				tr[symI]--
				dt++
			}
		}
		mbt = mb
	}

	out := make([]uint8, maxSym+1)
	for i := 0; i <= maxSym; i++ {
		out[i] = uint8(tr[i])
	}
	return out, mbt
}

// lc run-length encodes a Huffman code-length slice into symbols 0..18 packed with repeat counts
// and extra-bit widths, preserving fflate's empty-slice quirk (returns [0], 1).
func lc(c []uint8) ([]uint16, int) {
	s := len(c)
	for s > 0 {
		s--
		if c[s] != 0 {
			break
		}
	}
	s++
	cl := make([]uint16, s)
	cli := 0
	var cln uint16
	if len(c) > 0 {
		cln = uint16(c[0])
	}
	cls := 1
	for i := 1; i <= s; i++ {
		var ci uint16
		if i < s {
			ci = uint16(c[i])
		}
		if i != s && ci == cln {
			cls++
		} else {
			if cln == 0 && cls > 2 {
				for ; cls > 138; cls -= 138 {
					cl[cli] = 32754
					cli++
				}
				if cls > 2 {
					if cls > 10 {
						cl[cli] = uint16(((cls - 11) << 5) | 28690)
					} else {
						cl[cli] = uint16(((cls - 3) << 5) | 12305)
					}
					cli++
					cls = 0
				}
			} else if cls > 3 {
				cl[cli] = cln
				cli++
				cls--
				for ; cls > 6; cls -= 6 {
					cl[cli] = 8304
					cli++
				}
				if cls > 2 {
					cl[cli] = uint16(((cls - 3) << 5) | 8208)
					cli++
					cls = 0
				}
			}
			for cls > 0 {
				cl[cli] = cln
				cli++
				cls--
			}
			cls = 1
			cln = ci
		}
	}
	return cl[:cli], s
}

// clen computes sum(cf[i] * cl[i]).
func clen(cf []uint16, cl []uint8) int {
	l := 0
	for i := 0; i < len(cl); i++ {
		l += int(cf[i]) * int(cl[i])
	}
	return l
}
