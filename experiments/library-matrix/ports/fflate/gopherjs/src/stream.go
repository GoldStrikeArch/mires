// Synchronous streaming classes: Deflate, Gzip, Zlib, Inflate, Gunzip, Unzlib, and Decompress.
//
// All streams emit into a concrete *streamSink (never a func value or interface) so GopherJS
// compiles every streaming method without $restore state machines.
package main

type streamRecord struct {
	chunk []byte
	final bool
}

type streamSink struct {
	enabled bool
	records []*streamRecord
}

func (s *streamSink) emit(chunk []byte, final bool) {
	cp := make([]byte, len(chunk))
	copy(cp, chunk)
	s.records = append(s.records, &streamRecord{chunk: cp, final: final})
}

func (s *streamSink) frame() []byte {
	total := 0
	for _, r := range s.records {
		total += 5 + len(r.chunk)
	}
	out := make([]byte, total)
	off := 0
	for _, r := range s.records {
		n := len(r.chunk)
		out[off] = byte(n)
		out[off+1] = byte(n >> 8)
		out[off+2] = byte(n >> 16)
		out[off+3] = byte(n >> 24)
		if r.final {
			out[off+4] = 1
		}
		copy(out[off+5:], r.chunk)
		off += 5 + n
	}
	return out
}

const (
	streamDeflate = iota
	streamGzip
	streamZlib
)

type deflateStream struct {
	mode int
	o    compressOptions
	s    deflateState
	b    []byte
	sink *streamSink
	crc  crc32State
	l    uint32
	adl  adler32State
	v    int
}

func newDeflateStream(mode int, opts *compressOptions, sink *streamSink) *deflateStream {
	var o compressOptions
	if opts != nil {
		o = *opts
	}
	ds := &deflateStream{
		mode: mode,
		o:    o,
		s:    deflateState{l: 0, i: 32768, w: 32768, z: 32768},
		b:    make([]byte, 98304),
		sink: sink,
		crc:  newCRC32(),
		adl:  newAdler32(),
		v:    1,
	}
	if o.dictionary != nil {
		dict := o.dictionary
		if len(dict) > 32768 {
			dict = dict[len(dict)-32768:]
		}
		copy(ds.b[32768-len(dict):], dict)
		ds.s.i = 32768 - len(dict)
	}
	return ds
}

func (ds *deflateStream) p(c []byte, f bool) {
	switch ds.mode {
	case streamDeflate:
		raw := dopt(c, &ds.o, 0, 0, &ds.s)
		ds.sink.emit(raw, f)
	case streamGzip:
		pre := 0
		if ds.v != 0 {
			pre = gzhl(&ds.o)
		}
		post := 0
		if f {
			post = 8
		}
		raw := dopt(c, &ds.o, pre, post, &ds.s)
		if ds.v != 0 {
			gzh(raw, &ds.o)
			ds.v = 0
		}
		if f {
			wbytes(raw, len(raw)-8, ds.crc.digest())
			wbytes(raw, len(raw)-4, ds.l)
		}
		ds.sink.emit(raw, f)
	case streamZlib:
		pre := 0
		if ds.v != 0 {
			if ds.o.dictionary != nil {
				pre = 6
			} else {
				pre = 2
			}
		}
		post := 0
		if f {
			post = 4
		}
		raw := dopt(c, &ds.o, pre, post, &ds.s)
		if ds.v != 0 {
			zlh(raw, &ds.o)
			ds.v = 0
		}
		if f {
			wbytes(raw, len(raw)-4, ds.adl.digest())
		}
		ds.sink.emit(raw, f)
	}
}

func (ds *deflateStream) push(chunk []byte, final bool) *flateError {
	if ds.mode == streamGzip {
		ds.crc.push(chunk)
		ds.l += uint32(len(chunk))
	} else if ds.mode == streamZlib {
		ds.adl.push(chunk)
	}
	if !ds.sink.enabled {
		return newFlateError(errCodeNoStreamHandler, "no stream handler")
	}
	if ds.s.l != 0 {
		return newFlateError(errCodeStreamFinished, "stream finished")
	}
	endLen := len(chunk) + ds.s.z
	if endLen > len(ds.b) {
		if endLen > 2*len(ds.b)-32768 {
			newBuf := make([]byte, endLen & ^32767)
			copy(newBuf, ds.b[:ds.s.z])
			ds.b = newBuf
		}
		split := len(ds.b) - ds.s.z
		copy(ds.b[ds.s.z:], chunk[:split])
		ds.s.z = len(ds.b)
		ds.p(ds.b, false)
		copy(ds.b, ds.b[len(ds.b)-32768:])
		copy(ds.b[32768:], chunk[split:])
		ds.s.z = len(chunk) - split + 32768
		ds.s.i = 32766
		ds.s.w = 32768
	} else {
		copy(ds.b[ds.s.z:], chunk)
		ds.s.z += len(chunk)
	}
	if final {
		ds.s.l = 1
	} else {
		ds.s.l = 0
	}
	if ds.s.z > ds.s.w+8191 || final {
		ds.p(ds.b, final)
		ds.s.w = ds.s.i
		ds.s.i -= 2
	}
	if final {
		ds.s = deflateState{}
		ds.o = compressOptions{}
		ds.b = make([]byte, 0)
	}
	return nil
}

func (ds *deflateStream) flush(sync bool) *flateError {
	if !ds.sink.enabled {
		return newFlateError(errCodeNoStreamHandler, "no stream handler")
	}
	if ds.s.l != 0 {
		return newFlateError(errCodeStreamFinished, "stream finished")
	}
	ds.p(ds.b, false)
	ds.s.w = ds.s.i
	ds.s.i -= 2
	if sync {
		var c [6]byte
		c[0] = byte(ds.s.r >> 3)
		ep := wfblk(c[:], ds.s.r, nil)
		ds.s.r = 0
		ds.sink.emit(c[:ep>>3], false)
	}
	return nil
}

type inflateStream struct {
	s    inflateState
	o    []byte
	p    []byte
	d    bool
	sink *streamSink
}

func newInflateStream(opts *decompressOptions, sink *streamSink) *inflateStream {
	is := &inflateStream{
		o:    make([]byte, 32768),
		p:    make([]byte, 0),
		sink: sink,
	}
	if opts != nil && opts.dictionary != nil {
		dict := opts.dictionary
		if len(dict) > 32768 {
			dict = dict[len(dict)-32768:]
		}
		copy(is.o, dict)
		is.s.b = len(dict)
	}
	return is
}

func (is *inflateStream) e(c []byte) *flateError {
	if !is.sink.enabled {
		return newFlateError(errCodeNoStreamHandler, "no stream handler")
	}
	if is.d {
		return newFlateError(errCodeStreamFinished, "stream finished")
	}
	if len(is.p) == 0 {
		is.p = c
	} else if len(c) > 0 {
		n := make([]byte, len(is.p)+len(c))
		copy(n, is.p)
		copy(n[len(is.p):], c)
		is.p = n
	}
	return nil
}

func (is *inflateStream) c(final bool) *flateError {
	is.d = final
	if final {
		is.s.i = 1
	} else {
		is.s.i = 0
	}
	bts := is.s.b
	dt, err := inflt(is.p, &is.s, is.o, nil)
	if err != nil {
		return err
	}
	is.sink.emit(slc(dt, bts, is.s.b), is.d)
	is.o = slc(dt, is.s.b-32768, len(dt))
	is.s.b = len(is.o)
	is.p = slc(is.p, is.s.p>>3, len(is.p))
	is.s.p &= 7
	return nil
}

func (is *inflateStream) push(chunk []byte, final bool) *flateError {
	if err := is.e(chunk); err != nil {
		return err
	}
	return is.c(final)
}

type gunzipStream struct {
	base inflateStream
	v    int
	r    int
}

func newGunzipStream(opts *decompressOptions, sink *streamSink) *gunzipStream {
	return &gunzipStream{
		base: *newInflateStream(opts, sink),
		v:    1,
		r:    0,
	}
}

func (gs *gunzipStream) push(chunk []byte, final bool) *flateError {
	if err := gs.base.e(chunk); err != nil {
		return err
	}
	gs.r += len(chunk)
	if gs.v != 0 {
		start := gs.v - 1
		var p []byte
		if start < len(gs.base.p) {
			p = gs.base.p[start:]
		} else {
			p = make([]byte, 0)
		}
		s := 4
		if len(p) > 3 {
			var err *flateError
			s, err = gzs(p)
			if err != nil {
				return err
			}
		}
		if s > len(p) {
			if !final {
				return nil
			}
		}
		if s < len(p) {
			gs.base.p = p[s:]
		} else {
			gs.base.p = make([]byte, 0)
		}
		gs.v = 0
	}
	if err := gs.base.c(false); err != nil {
		return err
	}
	if gs.base.s.f != 0 && gs.base.s.l == nil {
		gs.v = shft(gs.base.s.p) + 9
		gs.base.s = inflateState{i: 0}
		gs.base.o = make([]byte, 0)
		return gs.push(make([]byte, 0), final)
	} else if final {
		return gs.base.c(true)
	}
	return nil
}

type unzlibStream struct {
	base inflateStream
	v    int
}

func newUnzlibStream(opts *decompressOptions, sink *streamSink) *unzlibStream {
	v := 1
	if opts != nil && opts.dictionary != nil {
		v = 2
	}
	return &unzlibStream{
		base: *newInflateStream(opts, sink),
		v:    v,
	}
}

func (uz *unzlibStream) push(chunk []byte, final bool) *flateError {
	if err := uz.base.e(chunk); err != nil {
		return err
	}
	if uz.v != 0 {
		if len(uz.base.p) < 6 && !final {
			return nil
		}
		st, err := zls(uz.base.p, uz.v-1 != 0)
		if err != nil {
			return err
		}
		if st < len(uz.base.p) {
			uz.base.p = uz.base.p[st:]
		} else {
			uz.base.p = make([]byte, 0)
		}
		uz.v = 0
	}
	if final {
		if len(uz.base.p) < 4 {
			return newFlateError(errCodeInvalidHeader, "invalid zlib data")
		}
		uz.base.p = uz.base.p[:len(uz.base.p)-4]
	}
	return uz.base.c(final)
}

const (
	decNone = iota
	decGunzip
	decInflate
	decUnzlib
)

type decompressStream struct {
	o    *decompressOptions
	sink *streamSink
	p    []byte
	kind int
	gz   *gunzipStream
	inf  *inflateStream
	uz   *unzlibStream
}

func newDecompressStream(opts *decompressOptions, sink *streamSink) *decompressStream {
	return &decompressStream{
		o:    opts,
		sink: sink,
		kind: decNone,
	}
}

func (ds *decompressStream) push(chunk []byte, final bool) *flateError {
	if !ds.sink.enabled {
		return newFlateError(errCodeNoStreamHandler, "no stream handler")
	}
	if ds.kind == decNone {
		if len(ds.p) > 0 {
			n := make([]byte, len(ds.p)+len(chunk))
			copy(n, ds.p)
			copy(n[len(ds.p):], chunk)
			ds.p = n
		} else {
			ds.p = chunk
		}
		if len(ds.p) > 2 {
			p := ds.p
			ds.p = nil
			if p[0] == 31 && p[1] == 139 && p[2] == 8 {
				ds.kind = decGunzip
				ds.gz = newGunzipStream(ds.o, ds.sink)
				return ds.gz.push(p, final)
			} else if (p[0]&15) != 8 || (p[0]>>4) > 7 || (((int(p[0])<<8)|int(p[1]))%31) != 0 {
				ds.kind = decInflate
				ds.inf = newInflateStream(ds.o, ds.sink)
				return ds.inf.push(p, final)
			} else {
				ds.kind = decUnzlib
				ds.uz = newUnzlibStream(ds.o, ds.sink)
				return ds.uz.push(p, final)
			}
		}
		return nil
	}
	switch ds.kind {
	case decGunzip:
		return ds.gz.push(chunk, final)
	case decInflate:
		return ds.inf.push(chunk, final)
	default:
		return ds.uz.push(chunk, final)
	}
}
