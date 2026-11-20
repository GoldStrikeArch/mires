// SHA3-256 and Keccak-256, adapted from @noble/hashes 2.4.0 (MIT).
package main

var sha3Pi = [...]int{20, 14, 22, 34, 36, 6, 10, 32, 16, 42, 48, 8, 30, 46, 38, 26, 24, 4, 40, 28, 44, 18, 12, 2}
var sha3Rotations = [...]int{1, 3, 6, 10, 15, 21, 28, 36, 45, 55, 2, 14, 27, 41, 56, 8, 25, 43, 62, 18, 39, 61, 20, 44}
var sha3RcLo = [...]uint32{1, 32898, 32906, 2147516416, 32907, 2147483649, 2147516545, 32777, 138, 136, 2147516425, 2147483658, 2147516555, 139, 32905, 32771, 32770, 128, 32778, 2147483658, 2147516545, 32896, 2147483649, 2147516424}
var sha3RcHi = [...]uint32{0, 0, 2147483648, 2147483648, 0, 0, 2147483648, 2147483648, 0, 0, 0, 0, 0, 2147483648, 2147483648, 2147483648, 2147483648, 2147483648, 0, 2147483648, 2147483648, 2147483648, 0, 2147483648}

// The 25 little-endian lanes occupy 50 words.
func sha3Permute(s []uint32) {
	columns := make([]uint32, 10)
	for round := 0; round < 24; round++ {
		// Theta mixes column parities into every lane.
		for x := 0; x < 10; x++ {
			columns[x] = s[x] ^ s[x+10] ^ s[x+20] ^ s[x+30] ^ s[x+40]
		}
		for col := 0; col < 5; col++ {
			x := 2 * col
			left := (x + 8) % 10
			right := (x + 2) % 10
			low := columns[left] ^ ((columns[right] << 1) | (columns[right+1] >> 31))
			high := columns[left+1] ^ ((columns[right+1] << 1) | (columns[right] >> 31))
			for row := 0; row < 5; row++ {
				i := row*10 + x
				s[i] = s[i] ^ low
				s[i+1] = s[i+1] ^ high
			}
		}
		// Rho rotates and Pi moves the 24 non-origin lanes in a single cycle.
		low := s[2]
		high := s[3]
		for t := 0; t < 24; t++ {
			shift := sha3Rotations[t]
			k := shift & 31
			a, b := low, high
			if shift > 32 {
				a, b = high, low
			}
			p := sha3Pi[t]
			low = s[p]
			high = s[p+1]
			s[p] = ((a << k) | (b >> (32 - k)))
			s[p+1] = ((b << k) | (a >> (32 - k)))
		}
		// Chi reads the original five lanes in each row.
		for row := 0; row < 5; row++ {
			y := 10 * row
			saved0 := s[y]
			saved1 := s[y+1]
			saved2 := s[y+2]
			saved3 := s[y+3]
			for x := 0; x < 6; x++ {
				s[y+x] = s[y+x] ^ (^s[y+x+2] & s[y+x+4])
			}
			s[y+6] = s[y+6] ^ (^s[y+8] & saved0)
			s[y+7] = s[y+7] ^ (^s[y+9] & saved1)
			s[y+8] = s[y+8] ^ (^saved0 & saved2)
			s[y+9] = s[y+9] ^ (^saved1 & saved3)
		}
		s[0] = s[0] ^ sha3RcLo[round]
		s[1] = s[1] ^ sha3RcHi[round]
	}
}
func sha3Digest(legacy bool, chunks [][]byte) []byte {
	s := make([]uint32, 50)
	pos := 0
	for _, chunk := range chunks {
		index := 0
		for index < len(chunk) {
			if pos == 0 && len(chunk)-index >= 136 {
				for word := 0; word < 34; word++ {
					i := index + 4*word
					s[word] = s[word] ^ (uint32(chunk[i]) | (uint32(chunk[i+1]) << 8) | (uint32(chunk[i+2]) << 16) | (uint32(chunk[i+3]) << 24))
				}
				index += 136
				sha3Permute(s)
			} else {
				s[pos/4] = s[pos/4] ^ (uint32(chunk[index]) << (8 * (pos % 4)))
				index++
				pos++
				if pos == 136 {
					sha3Permute(s)
					pos = 0
				}
			}
		}
	}
	suffix := uint32(6)
	if legacy {
		suffix = 1
	}
	s[pos/4] = s[pos/4] ^ (suffix << (8 * (pos % 4)))
	s[33] = s[33] ^ 0x80000000
	sha3Permute(s)
	result := make([]byte, 32)
	for i := 0; i < 32; i++ {
		result[i] = byte((s[i/4] >> (8 * (i % 4))))
	}
	return result
}
