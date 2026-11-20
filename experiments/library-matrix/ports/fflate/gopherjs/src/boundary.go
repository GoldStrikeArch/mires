// Boundary between JavaScript corpus inputs and the Go fflate implementation, mirroring
// libraries/fflate/reference.mjs.
package main

import "github.com/gopherjs/gopherjs/js"

func strToU8JS(textObj *js.Object, latin1 bool) []byte {
	n := textObj.Length()
	if n == 0 {
		return make([]byte, 0)
	}
	if latin1 {
		out := make([]byte, n)
		for i := 0; i < n; i++ {
			out[i] = byte(textObj.Call("charCodeAt", i).Int())
		}
		return out
	}
	// Encode UTF-16 code units to UTF-8 directly in Go (matching fflate's strToU8). This avoids
	// both GopherJS's $internalize string-concatenation loop / lone-surrogate defect and
	// cross-compartment Uint8Array mismatch on Firefox Marionette's TextEncoder wrapper.
	out := make([]byte, n+(n>>1)+4)
	ai := 0
	for i := 0; i < n; i++ {
		if ai+4 > len(out) {
			next := make([]byte, ai+8+((n-i)<<1))
			copy(next, out[:ai])
			out = next
		}
		c := textObj.Call("charCodeAt", i).Int()
		if c < 128 {
			out[ai] = byte(c)
			ai++
		} else if c < 2048 {
			out[ai] = byte(192 | (c >> 6))
			out[ai+1] = byte(128 | (c & 63))
			ai += 2
		} else if c >= 0xD800 && c <= 0xDBFF && i+1 < n {
			lo := textObj.Call("charCodeAt", i+1).Int()
			if lo >= 0xDC00 && lo <= 0xDFFF {
				i++
				cp := 0x10000 + ((c - 0xD800) << 10) + (lo - 0xDC00)
				out[ai] = byte(240 | (cp >> 18))
				out[ai+1] = byte(128 | ((cp >> 12) & 63))
				out[ai+2] = byte(128 | ((cp >> 6) & 63))
				out[ai+3] = byte(128 | (cp & 63))
				ai += 4
				continue
			}
			out[ai] = byte(224 | (c >> 12))
			out[ai+1] = byte(128 | ((c >> 6) & 63))
			out[ai+2] = byte(128 | (c & 63))
			ai += 3
		} else {
			out[ai] = byte(224 | (c >> 12))
			out[ai+1] = byte(128 | ((c >> 6) & 63))
			out[ai+2] = byte(128 | (c & 63))
			ai += 3
		}
	}
	return out[:ai]
}

func readPayload(spec *js.Object) []byte {
	bytesObj := spec.Get("bytes")
	if bytesObj != js.Undefined && bytesObj != nil {
		return js.Global.Get("Uint8Array").New(bytesObj).Interface().([]byte)
	}
	return strToU8JS(spec.Get("text"), spec.Get("latin1").Bool())
}

func readCodeUnits(s *js.Object) []uint16 {
	n := s.Length()
	out := make([]uint16, n)
	for i := 0; i < n; i++ {
		out[i] = uint16(s.Call("charCodeAt", i).Int())
	}
	return out
}

func readCompressOptions(obj *js.Object) *compressOptions {
	if obj == js.Undefined || obj == nil {
		return nil
	}
	opts := &compressOptions{}
	if v := obj.Get("level"); v != js.Undefined && v != nil {
		opts.hasLevel = true
		opts.level = v.Int()
	}
	if v := obj.Get("mem"); v != js.Undefined && v != nil {
		opts.hasMem = true
		opts.mem = v.Int()
	}
	if v := obj.Get("dictionary"); v != js.Undefined && v != nil {
		opts.dictionary = readPayload(v)
	}
	if v := obj.Get("mtime"); v != js.Undefined && v != nil {
		opts.hasMtime = true
		opts.mtime = v.Float()
	}
	if v := obj.Get("filename"); v != js.Undefined && v != nil {
		opts.filename = readCodeUnits(v)
	}
	return opts
}

func readDecompressOptions(obj *js.Object) *decompressOptions {
	if obj == js.Undefined || obj == nil {
		return nil
	}
	opts := &decompressOptions{}
	if v := obj.Get("dictionary"); v != js.Undefined && v != nil {
		opts.dictionary = readPayload(v)
	}
	if v := obj.Get("outSize"); v != js.Undefined && v != nil {
		opts.out = make([]byte, v.Int())
	}
	return opts
}

func runCompressStream(mode int, input *js.Object) ([]byte, *flateError) {
	opts := readCompressOptions(input.Get("options"))
	sink := &streamSink{enabled: !input.Get("noHandler").Bool()}
	strm := newDeflateStream(mode, opts, sink)
	steps := input.Get("steps")
	n := steps.Length()
	for i := 0; i < n; i++ {
		step := steps.Index(i)
		action := step.Get("action").String()
		switch action {
		case "push":
			if err := strm.push(readPayload(step.Get("data")), step.Get("final").Bool()); err != nil {
				return nil, err
			}
		case "flush":
			if err := strm.flush(step.Get("sync").Bool()); err != nil {
				return nil, err
			}
		default:
			panic("unknown stream action " + action)
		}
	}
	return sink.frame(), nil
}

func runDecompressStream(op string, input *js.Object) ([]byte, *flateError) {
	opts := readDecompressOptions(input.Get("options"))
	sink := &streamSink{enabled: !input.Get("noHandler").Bool()}
	steps := input.Get("steps")
	n := steps.Length()

	switch op {
	case "Inflate":
		strm := newInflateStream(opts, sink)
		for i := 0; i < n; i++ {
			step := steps.Index(i)
			action := step.Get("action").String()
			if action != "push" {
				panic("unknown stream action " + action)
			}
			if err := strm.push(readPayload(step.Get("data")), step.Get("final").Bool()); err != nil {
				return nil, err
			}
		}
	case "Gunzip":
		strm := newGunzipStream(opts, sink)
		for i := 0; i < n; i++ {
			step := steps.Index(i)
			action := step.Get("action").String()
			if action != "push" {
				panic("unknown stream action " + action)
			}
			if err := strm.push(readPayload(step.Get("data")), step.Get("final").Bool()); err != nil {
				return nil, err
			}
		}
	case "Unzlib":
		strm := newUnzlibStream(opts, sink)
		for i := 0; i < n; i++ {
			step := steps.Index(i)
			action := step.Get("action").String()
			if action != "push" {
				panic("unknown stream action " + action)
			}
			if err := strm.push(readPayload(step.Get("data")), step.Get("final").Bool()); err != nil {
				return nil, err
			}
		}
	case "Decompress":
		strm := newDecompressStream(opts, sink)
		for i := 0; i < n; i++ {
			step := steps.Index(i)
			action := step.Get("action").String()
			if action != "push" {
				panic("unknown stream action " + action)
			}
			if err := strm.push(readPayload(step.Get("data")), step.Get("final").Bool()); err != nil {
				return nil, err
			}
		}
	}
	return sink.frame(), nil
}

func executeOp(op string, input *js.Object) ([]byte, *flateError) {
	switch op {
	case "deflateSync":
		return deflateSync(readPayload(input.Get("data")), readCompressOptions(input.Get("options"))), nil
	case "inflateSync":
		return inflateSync(readPayload(input.Get("data")), readDecompressOptions(input.Get("options")))
	case "gzipSync":
		return gzipSync(readPayload(input.Get("data")), readCompressOptions(input.Get("options"))), nil
	case "gunzipSync":
		return gunzipSync(readPayload(input.Get("data")), readDecompressOptions(input.Get("options")))
	case "zlibSync":
		return zlibSync(readPayload(input.Get("data")), readCompressOptions(input.Get("options"))), nil
	case "unzlibSync":
		return unzlibSync(readPayload(input.Get("data")), readDecompressOptions(input.Get("options")))
	case "decompressSync":
		return decompressSync(readPayload(input.Get("data")), readDecompressOptions(input.Get("options")))
	case "strToU8":
		return strToU8JS(input.Get("text"), input.Get("latin1").Bool()), nil
	case "strFromU8":
		return strFromU8Utf16LE(readPayload(input.Get("data")), input.Get("latin1").Bool()), nil
	case "Deflate":
		return runCompressStream(streamDeflate, input)
	case "Gzip":
		return runCompressStream(streamGzip, input)
	case "Zlib":
		return runCompressStream(streamZlib, input)
	case "Inflate", "Gunzip", "Unzlib", "Decompress":
		return runDecompressStream(op, input)
	default:
		panic("unknown operation " + op)
	}
}

// run is the module's export: executes one corpus input and returns a non-nil []byte, which
// GopherJS externalises zero-copy as a Uint8Array subarray of the exact slice length.
func run(input *js.Object) []byte {
	op := input.Get("op").String()
	expectErr := input.Get("expectErrorCode").Bool()
	out, err := executeOp(op, input)
	if expectErr {
		if err != nil {
			return []byte{0xFF, byte(err.code)}
		}
		panic("expected FlateError for op " + op + ", but call succeeded")
	}
	if err != nil {
		panic(err.msg)
	}
	if out == nil {
		return make([]byte, 0)
	}
	return out
}
