// Boundary between JavaScript corpus inputs and the Go fflate implementation, mirroring
// libraries/fflate/reference.mjs.
package main

import "github.com/gopherjs/gopherjs/js"

// strToU8JS is fflate's strToU8: the low byte of each UTF-16 code unit for latin1, and otherwise
// the host TextEncoder's UTF-8, which fflate uses wherever the platform has one.
func strToU8JS(textObj *js.Object, latin1 bool) []byte {
	if !latin1 {
		return js.Global.Get("TextEncoder").New().Call("encode", textObj).Interface().([]byte)
	}
	n := textObj.Length()
	out := make([]byte, n)
	for i := 0; i < n; i++ {
		out[i] = byte(textObj.Call("charCodeAt", i).Int())
	}
	return out
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
