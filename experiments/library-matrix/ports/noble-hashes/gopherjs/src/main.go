// Command main is the boundary: libraries/noble-hashes/reference.mjs in Go. The input is the
// corpus object itself (*js.Object); byte arrays are copied into []byte.
package main

import "github.com/gopherjs/gopherjs/js"

// toBytes is Uint8Array.from(values).
func toBytes(values *js.Object) []byte {
	out := make([]byte, values.Length())
	for i := range out {
		out[i] = byte(values.Index(i).Int())
	}
	return out
}

// optionalBytes is toBytes of a field that may be absent.
func optionalBytes(input *js.Object, field string) []byte {
	if v := input.Get(field); v != js.Undefined {
		return toBytes(v)
	}
	return nil
}

// hasher is reference.mjs's hashes table plus HMAC-SHA-256: the constructor for algorithm.
func hasher(algorithm string) (func(blake2Opts) (hash, error), error) {
	switch algorithm {
	case "sha256":
		return func(blake2Opts) (hash, error) { return newSHA256(), nil }, nil
	case "sha512":
		return func(blake2Opts) (hash, error) { return newSHA512(), nil }, nil
	case "sha3_256":
		return func(blake2Opts) (hash, error) { return newKeccak(136, 0x06, 32), nil }, nil
	case "keccak_256":
		return func(blake2Opts) (hash, error) { return newKeccak(136, 0x01, 32), nil }, nil
	case "blake2b":
		return func(opts blake2Opts) (hash, error) { return newBlake2b(opts) }, nil
	case "blake2s":
		return func(opts blake2Opts) (hash, error) { return newBlake2s(opts) }, nil
	case "hmac_sha256":
		return func(opts blake2Opts) (hash, error) {
			if opts.key == nil {
				return nil, hashError(`"key" expected Uint8Array`)
			}
			return newHMAC(func() hash { return newSHA256() }, opts.key), nil
		}, nil
	}
	return nil, hashError("unknown hash")
}

func digestInput(input *js.Object) (string, error) {
	create, err := hasher(input.Get("algorithm").String())
	if err != nil {
		return "", err
	}
	opts := blake2Opts{
		key:             optionalBytes(input, "key"),
		salt:            optionalBytes(input, "salt"),
		personalization: optionalBytes(input, "personalization"),
	}
	if v := input.Get("dkLen"); v != js.Undefined {
		dkLen := v.Float()
		opts.dkLen = &dkLen
	}
	if bytes := input.Get("bytes"); bytes != js.Undefined {
		message := toBytes(bytes)
		h, err := create(opts)
		if err != nil {
			return "", err
		}
		h.update(message)
		return bytesToHex(h.digest()), nil
	}
	h, err := create(opts)
	if err != nil {
		return "", err
	}
	chunks := input.Get("chunks")
	blocks := make([][]byte, chunks.Length())
	for i := range blocks {
		blocks[i] = toBytes(chunks.Index(i))
	}
	repeat := 1
	if v := input.Get("repeat"); v != js.Undefined {
		repeat = v.Int()
	}
	for i := 0; i < repeat; i++ {
		for _, block := range blocks {
			h.update(block)
		}
	}
	return bytesToHex(h.digest()), nil
}

// run throws the error a hash reports, as noble does.
func run(input *js.Object) string {
	hex, err := digestInput(input)
	if err != nil {
		panic(js.Global.Get("Error").New(err.Error()))
	}
	return hex
}

func main() { js.Module.Get("exports").Set("run", run) }
