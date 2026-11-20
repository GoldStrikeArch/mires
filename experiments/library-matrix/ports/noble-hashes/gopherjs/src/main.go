package main

import "github.com/gopherjs/gopherjs/js"

func byteArray(value *js.Object) []byte {
	if value == js.Undefined {
		return nil
	}
	out := make([]byte, value.Length())
	for i := range out {
		out[i] = byte(value.Index(i).Int())
	}
	return out
}

func run(input *js.Object) string {
	algorithm := input.Get("algorithm").String()
	var chunks [][]byte
	if message := input.Get("bytes"); message != js.Undefined {
		chunks = [][]byte{byteArray(message)}
	} else {
		values := input.Get("chunks")
		blocks := make([][]byte, values.Length())
		for i := range blocks {
			blocks[i] = byteArray(values.Index(i))
		}
		n := 1
		if repeat := input.Get("repeat"); repeat != js.Undefined {
			n = repeat.Int()
		}
		for i := 0; i < n; i++ {
			chunks = append(chunks, blocks...)
		}
	}
	key := byteArray(input.Get("key"))
	var result []byte
	switch algorithm {
	case "sha256":
		result = sha2Digest(0, chunks, key)
	case "sha512":
		result = sha2Digest(1, chunks, key)
	case "hmac_sha256":
		if key == nil {
			panic("missing key")
		}
		result = sha2Digest(2, chunks, key)
	case "sha3_256", "keccak_256":
		result = sha3Digest(algorithm == "keccak_256", chunks)
	case "blake2b", "blake2s":
		wide := algorithm == "blake2b"
		max := 32
		if wide {
			max = 64
		}
		dk := max
		if size := input.Get("dkLen"); size != js.Undefined {
			dk = size.Int()
			if size.Float() != float64(dk) || dk < 1 || dk > max {
				panic("invalid digest length")
			}
		}
		salt, pers := byteArray(input.Get("salt")), byteArray(input.Get("personalization"))
		if (key != nil && (len(key) == 0 || len(key) > max)) || (salt != nil && len(salt) != max/4) || (pers != nil && len(pers) != max/4) {
			panic("invalid BLAKE2 parameters")
		}
		result = blake2Digest(wide, chunks, dk, key, salt, pers)
	default:
		panic("unknown algorithm")
	}
	const digits = "0123456789abcdef"
	out := make([]byte, 2*len(result))
	for i, b := range result {
		out[2*i] = digits[b>>4]
		out[2*i+1] = digits[b&15]
	}
	return string(out)
}
func main() { js.Module.Get("exports").Set("run", run) }
