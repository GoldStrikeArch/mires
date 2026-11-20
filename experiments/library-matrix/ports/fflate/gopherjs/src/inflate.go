// Raw DEFLATE decompressor state machine (inflt) and inflateSync, matching fflate 0.8.3
// bit-for-bit including zero-padded OOB bit reads, dynamic tree decoding, overlapping RLE copy,
// custom dictionary back-references, and non-resizing outSize truncation vs stored RangeError.
package main

type decompressOptions struct {
	dictionary []byte // non-nil when dictionary option was supplied
	out        []byte // non-nil when outSize option was supplied
}

type inflateState struct {
	l []uint16 // current lit/len decode map (nil between blocks)
	d []uint16 // current distance decode map
	m int      // lit/len max bits (lbt)
	n int      // distance max bits (dbt)
	f int      // BFINAL of current/last block (0 or 1)
	p int      // bit position in dat
	b int      // byte position in output buffer
	i int      // mode: 0 = non-final stream, 1 = final stream, 2 = one-shot sync
}

func maxU8(a []uint8) int {
	if len(a) == 0 {
		return 0
	}
	m := a[0]
	for i := 1; i < len(a); i++ {
		if a[i] > m {
			m = a[i]
		}
	}
	return int(m)
}

func slc(v []byte, s int, e int) []byte {
	if s < 0 {
		s = 0
	}
	if e > len(v) {
		e = len(v)
	}
	if s >= e {
		return make([]byte, 0)
	}
	out := make([]byte, e-s)
	copy(out, v[s:e])
	return out
}

func readBits(d []byte, p int, m int) int {
	o := p >> 3
	sl := len(d)
	var v uint32
	if o+1 < sl {
		v = uint32(d[o]) | (uint32(d[o+1]) << 8)
	} else if o < sl {
		v = uint32(d[o])
	}
	return int(v>>uint(p&7)) & m
}

func readBits16(d []byte, p int) uint32 {
	o := p >> 3
	sl := len(d)
	var v uint32
	if o+2 < sl {
		v = uint32(d[o]) | (uint32(d[o+1]) << 8) | (uint32(d[o+2]) << 16)
	} else if o < sl {
		v = uint32(d[o])
		if o+1 < sl {
			v |= uint32(d[o+1]) << 8
		}
	}
	return v >> uint(p&7)
}

func growBuf(buf []byte, l int) []byte {
	bl := len(buf)
	if l > bl {
		nlen := bl * 2
		if l > nlen {
			nlen = l
		}
		nbuf := make([]byte, nlen)
		copy(nbuf, buf)
		return nbuf
	}
	return buf
}

// inflt expands raw DEFLATE data according to st, writing into buf (if supplied) and resolving
// back-references into dict (if supplied).
func inflt(dat []byte, st *inflateState, buf []byte, dict []byte) ([]byte, *flateError) {
	sl := len(dat)
	dl := len(dict)
	if sl == 0 || (st.f != 0 && st.l == nil) {
		if buf != nil {
			return buf, nil
		}
		return make([]byte, 0), nil
	}
	noBuf := buf == nil
	resize := noBuf || st.i != 2
	noSt := st.i
	if noBuf {
		buf = make([]byte, sl*3)
	}

	final := st.f
	pos := st.p
	bt := st.b
	lm := st.l
	dm := st.d
	lbt := st.m
	dbt := st.n
	tbts := sl * 8
	sl2 := sl - 2

	for {
		if lm == nil {
			final = readBits(dat, pos, 1)
			btype := readBits(dat, pos+1, 3)
			pos += 3
			if btype == 0 {
				s := shft(pos) + 4
				l := 0
				if s-4 < sl {
					l = int(dat[s-4])
					if s-3 < sl {
						l |= int(dat[s-3]) << 8
					}
				}
				t := s + l
				if t > sl {
					if noSt != 0 {
						return nil, newFlateError(errCodeUnexpectedEOF, "unexpected EOF")
					}
					break
				}
				if resize {
					buf = growBuf(buf, bt+l)
				} else if bt+l > len(buf) {
					// Matches JS TypedArray.prototype.set throwing RangeError when !resize.
					panic("RangeError: offset is out of bounds")
				}
				copy(buf[bt:], dat[s:t])
				bt += l
				st.b = bt
				pos = t * 8
				st.p = pos
				st.f = final
				if final != 0 {
					break
				}
				continue
			} else if btype == 1 {
				lm = flrm
				dm = fdrm
				lbt = 9
				dbt = 5
			} else if btype == 2 {
				hLit := readBits(dat, pos, 31) + 257
				hcLen := readBits(dat, pos+10, 15) + 4
				tl := hLit + readBits(dat, pos+5, 31) + 1
				pos += 14
				ldt := make([]uint8, tl)
				var clt [19]uint8
				for i := 0; i < hcLen; i++ {
					clt[clim[i]] = uint8(readBits(dat, pos+i*3, 7))
				}
				pos += hcLen * 3
				clb := maxU8(clt[:])
				clbmsk := (1 << uint(clb)) - 1
				clm := hMap(clt[:], clb, true)
				for i := 0; i < tl; {
					r := int(clm[readBits(dat, pos, clbmsk)])
					pos += r & 15
					sym := r >> 4
					if sym < 16 {
						ldt[i] = uint8(sym)
						i++
					} else {
						var c uint8
						n := 0
						if sym == 16 {
							n = 3 + readBits(dat, pos, 3)
							pos += 2
							if i > 0 {
								c = ldt[i-1]
							}
						} else if sym == 17 {
							n = 3 + readBits(dat, pos, 7)
							pos += 3
						} else if sym == 18 {
							n = 11 + readBits(dat, pos, 127)
							pos += 7
						}
						for n > 0 {
							if i < tl {
								ldt[i] = c
							}
							i++
							n--
						}
					}
				}
				lt := ldt[:hLit]
				dtSlice := ldt[hLit:]
				lbt = maxU8(lt)
				dbt = maxU8(dtSlice)
				lm = hMap(lt, lbt, true)
				dm = hMap(dtSlice, dbt, true)
			} else {
				return nil, newFlateError(errCodeInvalidBlockType, "invalid block type")
			}
			if pos > tbts {
				if noSt != 0 {
					return nil, newFlateError(errCodeUnexpectedEOF, "unexpected EOF")
				}
				break
			}
		}

		if resize {
			buf = growBuf(buf, bt+131072)
		}
		lms := uint32((1 << uint(lbt)) - 1)
		dms := uint32((1 << uint(dbt)) - 1)
		lpos := pos

		for {
			o := pos >> 3
			var b16 uint32
			if o < sl2 {
				b16 = (uint32(dat[o]) | (uint32(dat[o+1]) << 8) | (uint32(dat[o+2]) << 16)) >> uint(pos&7)
			} else {
				b16 = readBits16(dat, pos)
			}
			c := int(lm[b16&lms])
			sym := c >> 4
			pos += c & 15
			if pos > tbts {
				if noSt != 0 {
					return nil, newFlateError(errCodeUnexpectedEOF, "unexpected EOF")
				}
				break
			}
			if c == 0 {
				return nil, newFlateError(errCodeInvalidLengthLiteral, "invalid length/literal")
			}
			if sym < 256 {
				if bt < len(buf) {
					buf[bt] = byte(sym)
				}
				bt++
			} else if sym == 256 {
				lpos = pos
				lm = nil
				break
			} else {
				add := sym - 254
				if sym > 264 {
					idx := sym - 257
					b := fleb[idx]
					o = pos >> 3
					var v uint32
					if o+1 < sl {
						v = (uint32(dat[o]) | (uint32(dat[o+1]) << 8)) >> uint(pos&7)
					} else if o < sl {
						v = uint32(dat[o]) >> uint(pos&7)
					}
					add = int(v&((uint32(1)<<b)-1)) + int(fl[idx])
					pos += int(b)
				}
				o = pos >> 3
				if o < sl2 {
					b16 = (uint32(dat[o]) | (uint32(dat[o+1]) << 8) | (uint32(dat[o+2]) << 16)) >> uint(pos&7)
				} else {
					b16 = readBits16(dat, pos)
				}
				d := int(dm[b16&dms])
				dsym := d >> 4
				if d == 0 {
					return nil, newFlateError(errCodeInvalidDistance, "invalid distance")
				}
				pos += d & 15
				dt := 0
				if dsym < 31 {
					dt = int(fd[dsym])
				}
				if dsym > 3 {
					b := fdeb[dsym]
					o = pos >> 3
					if o < sl2 {
						b16 = (uint32(dat[o]) | (uint32(dat[o+1]) << 8) | (uint32(dat[o+2]) << 16)) >> uint(pos&7)
					} else {
						b16 = readBits16(dat, pos)
					}
					dt += int(b16 & ((uint32(1) << b) - 1))
					pos += int(b)
				}
				if pos > tbts {
					if noSt != 0 {
						return nil, newFlateError(errCodeUnexpectedEOF, "unexpected EOF")
					}
					break
				}
				if resize {
					buf = growBuf(buf, bt+131072)
				}
				end := bt + add
				if bt < dt {
					shift := dl - dt
					dend := dt
					if end < dend {
						dend = end
					}
					if shift+bt < 0 {
						return nil, newFlateError(errCodeInvalidDistance, "invalid distance")
					}
					for ; bt < dend; bt++ {
						if bt < len(buf) {
							buf[bt] = dict[shift+bt]
						}
					}
				}
				if end <= len(buf) {
					for ; bt < end; bt++ {
						buf[bt] = buf[bt-dt]
					}
				} else {
					blen := len(buf)
					for ; bt < end; bt++ {
						if bt < blen {
							src := bt - dt
							if src < blen {
								buf[bt] = buf[src]
							} else {
								buf[bt] = 0
							}
						}
					}
				}
			}
			lpos = pos
		}

		st.l = lm
		st.p = lpos
		st.b = bt
		st.f = final
		if lm != nil {
			final = 1
			st.m = lbt
			st.d = dm
			st.n = dbt
		}
		if final != 0 {
			break
		}
	}

	if bt > len(buf) {
		bt = len(buf)
	}
	return buf[:bt], nil
}

func inflateSync(data []byte, opts *decompressOptions) ([]byte, *flateError) {
	var out, dict []byte
	if opts != nil {
		out = opts.out
		dict = opts.dictionary
	}
	return inflt(data, &inflateState{i: 2}, out, dict)
}
