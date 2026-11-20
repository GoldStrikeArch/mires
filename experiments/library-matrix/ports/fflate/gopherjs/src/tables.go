// Immutable lookup tables computed once at package initialisation, matching fflate 0.8.3's
// module-level tables (CONTRACT.md, "Fixed tables"). None of these tables is ever mutated after
// init(). No cross-call mutable state or input memoisation exists anywhere in the port.
package main

// Single package-level var block holding only immutable read-only lookup tables initialised once
// in init(), mirroring fflate's module-load tables.
var (
	// Fixed length extra bits (32 x uint8).
	fleb = [32]uint8{
		0, 0, 0, 0, 0, 0, 0, 0,
		1, 1, 1, 1, 2, 2, 2, 2,
		3, 3, 3, 3, 4, 4, 4, 4,
		5, 5, 5, 5, 0, 0, 0, 0,
	}
	// Fixed distance extra bits (32 x uint8).
	fdeb = [32]uint8{
		0, 0, 0, 0, 1, 1, 2, 2,
		3, 3, 4, 4, 5, 5, 6, 6,
		7, 7, 8, 8, 9, 9, 10, 10,
		11, 11, 12, 12, 13, 13, 0, 0,
	}
	// Code-length alphabet order (19 x uint8).
	clim = [19]uint8{
		16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
	}
	// Deflate level options: (nice << 13) | chain for levels 1..9.
	deo = [9]int32{
		65540, 131080, 131088, 131104, 262176, 1048704, 1048832, 2114560, 2117632,
	}

	// Length and distance base + reverse tables (populated once in init).
	fl    [31]uint16
	revfl [261]int32
	fd    [31]uint16
	revfd [32769]int32

	// 15-bit reversal table for 0..32767 (populated once in init).
	rev [32768]uint16

	// Fixed Huffman code lengths and encode/decode maps (populated once in init).
	flt  [288]uint8
	fdt  [32]uint8
	flm  []uint16
	flrm []uint16
	fdm  []uint16
	fdrm []uint16

	// Standard IEEE reflected CRC-32 lookup table (polynomial 0xEDB88320).
	crct [256]uint32
)

func init() {
	// Length base and reverse table: freb(fleb, 2), then fl[28] = 258, revfl[258] = 28.
	cur := 3
	fl[0] = 3
	for i := 1; i < 31; i++ {
		cur += 1 << fleb[i-1]
		fl[i] = uint16(cur)
	}
	for i := 1; i < 30; i++ {
		bi := int(fl[i])
		bn := int(fl[i+1])
		for j := bi; j < bn; j++ {
			revfl[j] = int32(((j - bi) << 5) | i)
		}
	}
	fl[28] = 258
	revfl[258] = 28

	// Distance base and reverse table: freb(fdeb, 0).
	cur = 1
	fd[0] = 1
	for i := 1; i < 31; i++ {
		cur += 1 << fdeb[i-1]
		fd[i] = uint16(cur)
	}
	for i := 1; i < 30; i++ {
		bi := int(fd[i])
		bn := int(fd[i+1])
		for j := bi; j < bn; j++ {
			revfd[j] = int32(((j - bi) << 5) | i)
		}
	}

	// 15-bit bit-reversal table.
	for i := 0; i < 32768; i++ {
		x := ((i & 0xAAAA) >> 1) | ((i & 0x5555) << 1)
		x = ((x & 0xCCCC) >> 2) | ((x & 0x3333) << 2)
		x = ((x & 0xF0F0) >> 4) | ((x & 0x0F0F) << 4)
		rev[i] = uint16((((x & 0xFF00) >> 8) | ((x & 0x00FF) << 8)) >> 1)
	}

	// Fixed literal/length and distance Huffman code lengths and tables.
	for i := 0; i < 144; i++ {
		flt[i] = 8
	}
	for i := 144; i < 256; i++ {
		flt[i] = 9
	}
	for i := 256; i < 280; i++ {
		flt[i] = 7
	}
	for i := 280; i < 288; i++ {
		flt[i] = 8
	}
	for i := 0; i < 32; i++ {
		fdt[i] = 5
	}
	flm = hMap(flt[:], 9, false)
	flrm = hMap(flt[:], 9, true)
	fdm = hMap(fdt[:], 5, false)
	fdrm = hMap(fdt[:], 5, true)

	// CRC-32 table (IEEE 802.3 reflected polynomial 0xEDB88320).
	for i := 0; i < 256; i++ {
		c := uint32(i)
		for k := 0; k < 8; k++ {
			if (c & 1) != 0 {
				c = 0xEDB88320 ^ (c >> 1)
			} else {
				c >>= 1
			}
		}
		crct[i] = c
	}
}

// defaultPlvl is Math.ceil(Math.max(8, Math.min(13, Math.log(n))) * 1.5), computed by exact
// integer cutoffs so the port does not import math.
func defaultPlvl(n int) int {
	switch {
	case n <= 2980:
		return 12
	case n <= 5806:
		return 13
	case n <= 11308:
		return 14
	case n <= 22026:
		return 15
	case n <= 42901:
		return 16
	case n <= 83561:
		return 17
	case n <= 162754:
		return 18
	case n <= 317003:
		return 19
	default:
		return 20
	}
}
