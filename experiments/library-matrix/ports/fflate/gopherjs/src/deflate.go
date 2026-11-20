// Bit writers, block selection & emission (wfblk, wblk), and LZ77 DEFLATE compressor (dflt, dopt,
// deflateSync), matching fflate 0.8.3 bit-for-bit.
package main

type compressOptions struct {
	hasLevel   bool
	level      int
	hasMem     bool
	mem        int
	dictionary []byte // non-nil when dictionary option was supplied (even if empty)
	hasMtime   bool
	mtime      float64
	filename   []uint16 // UTF-16 code units; empty when omitted or ""
}

type deflateState struct {
	l int      // BFINAL (0 or 1)
	r int      // leftover bit count (low 3 bits) | partial byte (>> 3)
	i int      // hash index
	w int      // wait index
	z int      // input end (0 means len(dat))
	h []uint16 // head hash table
	p []uint16 // prev chain table
}

func shft(p int) int {
	return (p + 7) >> 3
}

func wbits(d []byte, p int, v int) {
	v <<= uint(p & 7)
	o := p >> 3
	d[o] |= byte(v)
	d[o+1] |= byte(v >> 8)
}

func wbits16(d []byte, p int, v int) {
	v <<= uint(p & 7)
	o := p >> 3
	d[o] |= byte(v)
	d[o+1] |= byte(v >> 8)
	d[o+2] |= byte(v >> 16)
}

// wfblk writes a stored (BTYPE=00) block starting after the 1-bit BFINAL already written at pos-1.
func wfblk(out []byte, pos int, dat []byte) int {
	s := len(dat)
	o := shft(pos + 2)
	out[o] = byte(s)
	out[o+1] = byte(s >> 8)
	out[o+2] = out[o] ^ 255
	out[o+3] = out[o+1] ^ 255
	copy(out[o+4:], dat)
	return (o + 4 + s) * 8
}

// wblk chooses among stored, fixed Huffman, and dynamic Huffman blocks and emits the chosen block.
func wblk(dat []byte, out []byte, final int, syms []int32, lf []uint16, df []uint16, eb int, li int, bs int, bl int, p int) int {
	wbits(out, p, final)
	p++
	lf[256]++
	dlt, mlb := hTree(lf, 15)
	ddt, mdb := hTree(df, 15)
	lclt, nlc := lc(dlt)
	lcdt, ndc := lc(ddt)
	var lcfreq [19]uint16
	for i := 0; i < len(lclt); i++ {
		lcfreq[lclt[i]&31]++
	}
	for i := 0; i < len(lcdt); i++ {
		lcfreq[lcdt[i]&31]++
	}
	lct, mlcb := hTree(lcfreq[:], 7)
	nlcc := 19
	for nlcc > 4 {
		idx := int(clim[nlcc-1])
		if idx < len(lct) && lct[idx] != 0 {
			break
		}
		nlcc--
	}
	flen := (bl + 5) << 3
	ftlen := clen(lf, flt[:]) + clen(df, fdt[:]) + eb
	dtlen := clen(lf, dlt) + clen(df, ddt) + eb + 14 + 3*nlcc +
		clen(lcfreq[:], lct) + 2*int(lcfreq[16]) + 3*int(lcfreq[17]) + 7*int(lcfreq[18])

	if bs >= 0 && flen <= ftlen && flen <= dtlen {
		return wfblk(out, p, dat[bs:bs+bl])
	}

	var lm, dm []uint16
	var ll, dl []uint8
	if dtlen < ftlen {
		wbits(out, p, 2)
		p += 2
		lm = hMap(dlt, mlb, false)
		ll = dlt
		dm = hMap(ddt, mdb, false)
		dl = ddt
		llm := hMap(lct, mlcb, false)
		wbits(out, p, nlc-257)
		wbits(out, p+5, ndc-1)
		wbits(out, p+10, nlcc-4)
		p += 14
		for i := 0; i < nlcc; i++ {
			idx := int(clim[i])
			if idx < len(lct) {
				wbits(out, p+3*i, int(lct[idx]))
			}
		}
		p += 3 * nlcc
		for it := 0; it < 2; it++ {
			clct := lclt
			if it == 1 {
				clct = lcdt
			}
			for i := 0; i < len(clct); i++ {
				v := int(clct[i])
				ln := v & 31
				wbits(out, p, int(llm[ln]))
				p += int(lct[ln])
				if ln > 15 {
					wbits(out, p, (v>>5)&127)
					p += v >> 12
				}
			}
		}
	} else {
		wbits(out, p, 1)
		p += 2
		lm = flm
		ll = flt[:]
		dm = fdm
		dl = fdt[:]
	}

	// Emit symbols with inlined bit writes on the hot path.
	for i := 0; i < li; i++ {
		sym := int(syms[i])
		if sym > 255 {
			ln := (sym >> 18) & 31
			v := int(lm[ln+257]) << uint(p&7)
			o := p >> 3
			out[o] |= byte(v)
			out[o+1] |= byte(v >> 8)
			out[o+2] |= byte(v >> 16)
			p += int(ll[ln+257])
			if ln > 7 {
				v = ((sym >> 23) & 31) << uint(p&7)
				o = p >> 3
				out[o] |= byte(v)
				out[o+1] |= byte(v >> 8)
				p += int(fleb[ln])
			}
			dst := sym & 31
			v = int(dm[dst]) << uint(p&7)
			o = p >> 3
			out[o] |= byte(v)
			out[o+1] |= byte(v >> 8)
			out[o+2] |= byte(v >> 16)
			p += int(dl[dst])
			if dst > 3 {
				v = ((sym >> 5) & 8191) << uint(p&7)
				o = p >> 3
				out[o] |= byte(v)
				out[o+1] |= byte(v >> 8)
				out[o+2] |= byte(v >> 16)
				p += int(fdeb[dst])
			}
		} else {
			v := int(lm[sym]) << uint(p&7)
			o := p >> 3
			out[o] |= byte(v)
			out[o+1] |= byte(v >> 8)
			out[o+2] |= byte(v >> 16)
			p += int(ll[sym])
		}
	}
	wbits16(out, p, int(lm[256]))
	return p + int(ll[256])
}

// dflt compresses dat into a raw DEFLATE buffer surrounded by pre header bytes and post trailer
// bytes. Two slack zero bytes are allocated past w so wbits16 never indexes past slice bounds.
func dflt(dat []byte, lvl int, plvl int, pre int, post int, st *deflateState) []byte {
	s := st.z
	if s == 0 {
		s = len(dat)
	}
	wSize := s + 5*(1+(s+6999)/7000)
	o := make([]byte, pre+wSize+post+2)
	w := o[pre : pre+wSize+2]
	lst := st.l
	pos := st.r & 7

	if lvl != 0 {
		if pos != 0 {
			w[0] = byte(st.r >> 3)
		}
		opt := int(deo[lvl-1])
		n := opt >> 13
		c := opt & 8191
		msk := (1 << uint(plvl)) - 1
		prev := st.p
		if prev == nil {
			prev = make([]uint16, 32768)
		}
		head := st.h
		if head == nil {
			head = make([]uint16, msk+1)
		}
		bs1 := uint((plvl + 2) / 3)
		bs2 := 2 * bs1

		syms := make([]int32, 25000)
		var lf [288]uint16
		var df [32]uint16
		lcCnt := 0
		eb := 0
		i := st.i
		li := 0
		wi := st.w
		bs := 0

		for ; i+2 < s; i++ {
			d0 := dat[i]
			hv := (int(d0) ^ (int(dat[i+1]) << bs1) ^ (int(dat[i+2]) << bs2)) & msk
			imod := i & 32767
			pimod := int(head[hv])
			prev[imod] = uint16(pimod)
			head[hv] = uint16(imod)

			if wi <= i {
				rem := s - i
				if (lcCnt > 7000 || li > 24576) && (rem > 423 || lst == 0) {
					pos = wblk(dat, w, 0, syms, lf[:], df[:], eb, li, bs, i-bs, pos)
					li = 0
					lcCnt = 0
					eb = 0
					bs = i
					for j := 0; j < 286; j++ {
						lf[j] = 0
					}
					for j := 0; j < 30; j++ {
						df[j] = 0
					}
				}
				l := 2
				d := 0
				ch := c
				dif := (imod - pimod) & 32767
				if rem > 2 && i >= dif && hv == ((int(dat[i-dif])^(int(dat[i-dif+1])<<bs1)^(int(dat[i-dif+2])<<bs2))&msk) {
					maxn := n
					if rem < maxn {
						maxn = rem
					}
					maxn--
					maxd := i
					if maxd > 32767 {
						maxd = 32767
					}
					ml := rem
					if ml > 258 {
						ml = 258
					}
					for dif <= maxd {
						ch--
						if ch == 0 || imod == pimod {
							break
						}
						if dat[i+l] == dat[i+l-dif] {
							nl := 0
							for nl < ml && dat[i+nl] == dat[i+nl-dif] {
								nl++
							}
							if nl > l {
								l = nl
								d = dif
								if nl > maxn {
									break
								}
								mmd := nl - 2
								if dif < mmd {
									mmd = dif
								}
								md := 0
								for j := 0; j < mmd; j++ {
									ti := (i - dif + j) & 32767
									pti := int(prev[ti])
									cd := (ti - pti) & 32767
									if cd > md {
										md = cd
										pimod = ti
									}
								}
							}
						}
						imod = pimod
						pimod = int(prev[imod])
						dif += (imod - pimod) & 32767
					}
				}
				if d != 0 {
					rl := int(revfl[l])
					rd := int(revfd[d])
					syms[li] = int32(268435456 | (rl << 18) | rd)
					li++
					lin := rl & 31
					din := rd & 31
					eb += int(fleb[lin]) + int(fdeb[din])
					lf[257+lin]++
					df[din]++
					wi = i + l
					lcCnt++
				} else {
					syms[li] = int32(d0)
					li++
					lf[d0]++
				}
			}
		}

		if wi > i {
			i = wi
		}
		for ; i < s; i++ {
			b := dat[i]
			syms[li] = int32(b)
			li++
			lf[b]++
		}
		pos = wblk(dat, w, lst, syms, lf[:], df[:], eb, li, bs, i-bs, pos)
		if lst == 0 {
			st.r = (pos & 7) | (int(w[pos>>3]) << 3)
			pos -= 7
			st.h = head
			st.p = prev
			st.i = i
			st.w = wi
		}
	} else {
		for i := st.w; i < s+lst; i += 65535 {
			e := i + 65535
			if e >= s {
				w[pos>>3] = byte(lst)
				e = s
			}
			pos = wfblk(w, pos+1, dat[i:e])
		}
		st.i = s
	}

	return o[:pre+shft(pos)+post]
}

// dopt resolves compression level, hash-table plvl, and one-shot dictionary prepending.
func dopt(dat []byte, opt *compressOptions, pre int, post int, st *deflateState) []byte {
	if st == nil {
		st = &deflateState{l: 1}
		if opt != nil && opt.dictionary != nil {
			dict := opt.dictionary
			if len(dict) > 32768 {
				dict = dict[len(dict)-32768:]
			}
			newDat := make([]byte, len(dict)+len(dat))
			copy(newDat, dict)
			copy(newDat[len(dict):], dat)
			dat = newDat
			st.w = len(dict)
		}
	}
	lvl := 6
	if opt != nil && opt.hasLevel {
		lvl = opt.level
	}
	var plvl int
	if opt != nil && opt.hasMem {
		plvl = 12 + opt.mem
	} else if st.l != 0 {
		plvl = defaultPlvl(len(dat))
	} else {
		plvl = 20
	}
	return dflt(dat, lvl, plvl, pre, post, st)
}

func deflateSync(data []byte, opts *compressOptions) []byte {
	return dopt(data, opts, 0, 0, nil)
}
