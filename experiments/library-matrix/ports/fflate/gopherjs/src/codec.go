// strFromU8 decoding to exact UTF-16LE bytes (CONTRACT.md, "String codec"):
//   - latin1 == true: each byte b -> UTF-16LE code unit [b, 0x00].
//   - latin1 == false: WHATWG Encoding Standard UTF-8 decoder (ignoreBOM: false, fatal: false):
//     strips at most one leading UTF-8 BOM (EF BB BF), encodes astral code points as UTF-16
//     surrogate pairs, and replaces each maximal subpart of an ill-formed sequence with U+FFFD.
package main

func strFromU8Utf16LE(dat []byte, latin1 bool) []byte {
	if latin1 {
		out := make([]byte, len(dat)*2)
		for i := 0; i < len(dat); i++ {
			out[i*2] = dat[i]
		}
		return out
	}

	start := 0
	if len(dat) >= 3 && dat[0] == 0xEF && dat[1] == 0xBB && dat[2] == 0xBF {
		start = 3
	}

	// Maximum UTF-16LE output length for valid or invalid UTF-8 is 2 bytes per input byte.
	out := make([]byte, (len(dat)-start)*2)
	oi := 0

	codePoint := 0
	bytesSeen := 0
	bytesNeeded := 0
	lowerBoundary := byte(0x80)
	upperBoundary := byte(0xBF)

	for i := start; i < len(dat); {
		b := dat[i]
		if bytesNeeded == 0 {
			if b <= 0x7F {
				out[oi] = b
				out[oi+1] = 0
				oi += 2
				i++
			} else if b >= 0xC2 && b <= 0xDF {
				bytesNeeded = 1
				codePoint = int(b & 0x1F)
				i++
			} else if b >= 0xE0 && b <= 0xEF {
				if b == 0xE0 {
					lowerBoundary = 0xA0
				} else if b == 0xED {
					upperBoundary = 0x9F
				}
				bytesNeeded = 2
				codePoint = int(b & 0x0F)
				i++
			} else if b >= 0xF0 && b <= 0xF4 {
				if b == 0xF0 {
					lowerBoundary = 0x90
				} else if b == 0xF4 {
					upperBoundary = 0x8F
				}
				bytesNeeded = 3
				codePoint = int(b & 0x07)
				i++
			} else {
				out[oi] = 0xFD
				out[oi+1] = 0xFF
				oi += 2
				i++
			}
		} else {
			if b < lowerBoundary || b > upperBoundary {
				codePoint = 0
				bytesNeeded = 0
				bytesSeen = 0
				lowerBoundary = 0x80
				upperBoundary = 0xBF
				out[oi] = 0xFD
				out[oi+1] = 0xFF
				oi += 2
				// Reprocess b as a new lead byte.
			} else {
				lowerBoundary = 0x80
				upperBoundary = 0xBF
				codePoint = (codePoint << 6) | int(b&0x3F)
				bytesSeen++
				i++
				if bytesSeen == bytesNeeded {
					cp := codePoint
					codePoint = 0
					bytesNeeded = 0
					bytesSeen = 0
					if cp <= 0xFFFF {
						out[oi] = byte(cp)
						out[oi+1] = byte(cp >> 8)
						oi += 2
					} else {
						cp -= 0x10000
						hi := 0xD800 | (cp >> 10)
						lo := 0xDC00 | (cp & 0x3FF)
						out[oi] = byte(hi)
						out[oi+1] = byte(hi >> 8)
						out[oi+2] = byte(lo)
						out[oi+3] = byte(lo >> 8)
						oi += 4
					}
				}
			}
		}
	}

	if bytesNeeded != 0 {
		out[oi] = 0xFD
		out[oi+1] = 0xFF
		oi += 2
	}

	return out[:oi]
}
