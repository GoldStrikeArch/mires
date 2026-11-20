// Checksums (CRC-32, Adler-32), little-endian wbytes, GZIP and Zlib headers/trailers,
// decompressSync auto-detection, and FlateError codes.
package main

import "github.com/gopherjs/gopherjs/js"

const (
	errCodeUnexpectedEOF        = 0
	errCodeInvalidBlockType     = 1
	errCodeInvalidLengthLiteral = 2
	errCodeInvalidDistance      = 3
	errCodeStreamFinished       = 4
	errCodeNoStreamHandler      = 5
	errCodeInvalidHeader        = 6
)

type flateError struct {
	code int
	msg  string
}

func newFlateError(code int, msg string) *flateError {
	return &flateError{code: code, msg: msg}
}

type crc32State struct {
	c uint32
}

func newCRC32() crc32State {
	return crc32State{c: 0xFFFFFFFF}
}

func (s *crc32State) push(d []byte) {
	cr := s.c
	for i := 0; i < len(d); i++ {
		cr = crct[(cr^uint32(d[i]))&255] ^ (cr >> 8)
	}
	s.c = cr
}

func (s *crc32State) digest() uint32 {
	return ^s.c
}

type adler32State struct {
	a uint32
	b uint32
}

func newAdler32() adler32State {
	return adler32State{a: 1, b: 0}
}

func (s *adler32State) push(d []byte) {
	n := s.a
	m := s.b
	l := len(d)
	for i := 0; i != l; {
		e := i + 2655
		if e > l {
			e = l
		}
		for ; i < e; i++ {
			n += uint32(d[i])
			m += n
		}
		n = (n & 65535) + 15*(n>>16)
		m = (m & 65535) + 15*(m>>16)
	}
	s.a = n
	s.b = m
}

func (s *adler32State) digest() uint32 {
	s.a %= 65521
	s.b %= 65521
	a := s.a
	b := s.b
	return ((a & 255) << 24) | ((a & 0xFF00) << 8) | ((b & 255) << 8) | (b >> 8)
}

func wbytes(d []byte, b int, v uint32) {
	for v != 0 {
		d[b] = byte(v)
		v >>= 8
		b++
	}
}

// GZIP header length.
func gzhl(o *compressOptions) int {
	if o != nil && len(o.filename) > 0 {
		return 10 + len(o.filename) + 1
	}
	return 10
}

// GZIP header writer.
func gzh(c []byte, o *compressOptions) {
	c[0] = 31
	c[1] = 139
	c[2] = 8
	if o != nil && o.hasLevel {
		if o.level < 2 {
			c[8] = 4
		} else if o.level == 9 {
			c[8] = 2
		} else {
			c[8] = 0
		}
	} else {
		c[8] = 0
	}
	c[9] = 3
	if o != nil && o.hasMtime {
		if o.mtime != 0 {
			sec := uint32(js.Global.Get("Math").Call("floor", o.mtime/1000.0).Float())
			wbytes(c, 4, sec)
		}
	} else {
		sec := uint32(js.Global.Get("Math").Call("floor", js.Global.Get("Date").Call("now").Float()/1000.0).Float())
		wbytes(c, 4, sec)
	}
	if o != nil && len(o.filename) > 0 {
		c[3] = 8
		for i := 0; i < len(o.filename); i++ {
			c[10+i] = byte(o.filename[i])
		}
		c[10+len(o.filename)] = 0
	}
}

// GZIP header parser: returns offset of raw DEFLATE payload.
func gzs(d []byte) (int, *flateError) {
	if len(d) < 3 || d[0] != 31 || d[1] != 139 || d[2] != 8 {
		return 0, newFlateError(errCodeInvalidHeader, "invalid gzip data")
	}
	var flg byte
	if len(d) > 3 {
		flg = d[3]
	}
	st := 10
	if (flg & 4) != 0 {
		xlen := 0
		if len(d) > 10 {
			xlen = int(d[10])
			if len(d) > 11 {
				xlen |= int(d[11]) << 8
			}
		}
		st += xlen + 2
	}
	zs := int((flg>>3)&1) + int((flg>>4)&1)
	for zs > 0 {
		var b byte
		if st < len(d) {
			b = d[st]
		}
		st++
		if b == 0 {
			zs--
		}
	}
	return st + int(flg&2), nil
}

// GZIP ISIZE trailer (last 4 bytes LE).
func gzl(d []byte) uint32 {
	l := len(d)
	if l < 4 {
		return 0
	}
	return uint32(d[l-4]) | (uint32(d[l-3]) << 8) | (uint32(d[l-2]) << 16) | (uint32(d[l-1]) << 24)
}

func gzipSync(data []byte, opts *compressOptions) []byte {
	c := newCRC32()
	l := uint32(len(data))
	c.push(data)
	d := dopt(data, opts, gzhl(opts), 8, nil)
	s := len(d)
	gzh(d, opts)
	wbytes(d, s-8, c.digest())
	wbytes(d, s-4, l)
	return d
}

func gunzipSync(data []byte, opts *decompressOptions) ([]byte, *flateError) {
	st, err := gzs(data)
	if err != nil {
		return nil, err
	}
	if st+8 > len(data) {
		return nil, newFlateError(errCodeInvalidHeader, "invalid gzip data")
	}
	var out, dict []byte
	if opts != nil {
		out = opts.out
		dict = opts.dictionary
	}
	if out == nil {
		sz := gzl(data)
		// GopherJS int is signed 32-bit; a corrupted ISIZE >= 2^31 would overflow makeslice.
		// Leave out nil so inflt allocates dynamically and hits the same deflate/error path as JS.
		if sz <= 0x7FFFFFFF {
			out = make([]byte, int(sz))
		}
	}
	return inflt(data[st:len(data)-8], &inflateState{i: 2}, out, dict)
}

// Zlib header writer.
func zlh(c []byte, o *compressOptions) {
	fl := 2
	if o != nil && o.hasLevel {
		if o.level == 0 {
			fl = 0
		} else if o.level < 6 {
			fl = 1
		} else if o.level == 9 {
			fl = 3
		}
	}
	c[0] = 120
	c1 := fl << 6
	if o != nil && o.dictionary != nil {
		c1 |= 32
	}
	c1 |= 31 - (((120 << 8) | c1) % 31)
	c[1] = byte(c1)
	if o != nil && o.dictionary != nil {
		h := newAdler32()
		h.push(o.dictionary)
		wbytes(c, 2, h.digest())
	}
}

// Zlib header parser: returns 6 if FDICT else 2.
func zls(d []byte, hasDict bool) (int, *flateError) {
	d0, d1 := 0, 0
	if len(d) > 0 {
		d0 = int(d[0])
	}
	if len(d) > 1 {
		d1 = int(d[1])
	}
	if (d0&15) != 8 || (d0>>4) > 7 || (((d0<<8)|d1)%31) != 0 {
		return 0, newFlateError(errCodeInvalidHeader, "invalid zlib data")
	}
	if (((d1 >> 5) & 1) == 1) != hasDict {
		return 0, newFlateError(errCodeInvalidHeader, "invalid zlib data: dictionary mismatch")
	}
	return ((d1 >> 3) & 4) + 2, nil
}

func zlibSync(data []byte, opts *compressOptions) []byte {
	a := newAdler32()
	a.push(data)
	pre := 2
	if opts != nil && opts.dictionary != nil {
		pre = 6
	}
	d := dopt(data, opts, pre, 4, nil)
	zlh(d, opts)
	wbytes(d, len(d)-4, a.digest())
	return d
}

func unzlibSync(data []byte, opts *decompressOptions) ([]byte, *flateError) {
	hasDict := opts != nil && opts.dictionary != nil
	st, err := zls(data, hasDict)
	if err != nil {
		return nil, err
	}
	end := len(data) - 4
	if end < st {
		end = st
	}
	var out, dict []byte
	if opts != nil {
		out = opts.out
		dict = opts.dictionary
	}
	return inflt(data[st:end], &inflateState{i: 2}, out, dict)
}

func decompressSync(data []byte, opts *decompressOptions) ([]byte, *flateError) {
	d0, d1, d2 := 0, 0, 0
	if len(data) > 0 {
		d0 = int(data[0])
	}
	if len(data) > 1 {
		d1 = int(data[1])
	}
	if len(data) > 2 {
		d2 = int(data[2])
	}
	if d0 == 31 && d1 == 139 && d2 == 8 {
		return gunzipSync(data, opts)
	}
	if (d0&15) != 8 || (d0>>4) > 7 || (((d0<<8)|d1)%31) != 0 {
		return inflateSync(data, opts)
	}
	return unzlibSync(data, opts)
}
