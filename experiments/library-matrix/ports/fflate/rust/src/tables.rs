//! Immutable lookup tables of fflate 0.8.3, computed at compile time (`const fn`) so they carry
//! zero cross-call mutable state and zero per-call initialisation cost.
//!
//! `rev15` and `revfd` are computed in O(1) instead of storing 32 KiB static literals.

/// Fixed length extra bits (RFC 1951 / fflate `fleb`).
pub static FLEB: [u8; 32] = [
    0, 0, 0, 0, 0, 0, 0, 0,
    1, 1, 1, 1, 2, 2, 2, 2,
    3, 3, 3, 3, 4, 4, 4, 4,
    5, 5, 5, 5, 0, 0, 0, 0,
];

/// Fixed distance extra bits (RFC 1951 / fflate `fdeb`).
pub static FDEB: [u8; 32] = [
    0, 0, 0, 0, 1, 1, 2, 2,
    3, 3, 4, 4, 5, 5, 6, 6,
    7, 7, 8, 8, 9, 9, 10, 10,
    11, 11, 12, 12, 13, 13, 0, 0,
];

/// Code-length alphabet permutation order (RFC 1951 / fflate `clim`).
pub static CLIM: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

/// Deflate level options for levels 1..=9: `(nice << 13) | chain` (fflate `deo`).
pub static DEO: [u32; 9] = [
    65540, 131080, 131088, 131104, 262176, 1048704, 1048832, 2114560, 2117632,
];

/// Fixed length base table (fflate `fl`).
pub static FL: [u16; 31] = make_fl();

/// Reverse length lookup table: `revfl[len] = ((len - fl[i]) << 5) | i`, with `revfl[258] = 28`.
pub static REVFL: [u16; 259] = make_revfl();

/// Fixed distance base table (fflate `fd`).
pub static FD: [u16; 31] = make_fd();

/// Fixed literal/length code lengths (fflate `flt`).
pub static FLT: [u8; 288] = make_flt();

/// Fixed distance code lengths (fflate `fdt`).
pub static FDT: [u8; 32] = [5; 32];

/// Fixed literal/length forward Huffman map (`hMap(flt, 9, 0)`).
pub static FLM: [u16; 288] = make_fixed_forward_288();

/// Fixed literal/length reverse Huffman map (`hMap(flt, 9, 1)`).
pub static FLRM: [u16; 512] = make_fixed_reverse_512();

/// Fixed distance forward Huffman map (`hMap(fdt, 5, 0)`).
pub static FDM: [u16; 32] = make_fixed_forward_32();

/// Fixed distance reverse Huffman map (`hMap(fdt, 5, 1)`).
pub static FDRM: [u16; 32] = make_fixed_reverse_32();

/// Reflected IEEE CRC-32 table (polynomial `0xEDB88320`, fflate `crct`).
pub static CRCT: [u32; 256] = make_crct();

/// Reverses the low 15 bits of `x` (equivalent to `rev[x]` in fflate for `0 <= x < 32768`).
#[inline]
pub const fn rev15(mut x: u32) -> u32 {
    x = ((x & 0xAAAA) >> 1) | ((x & 0x5555) << 1);
    x = ((x & 0xCCCC) >> 2) | ((x & 0x3333) << 2);
    x = ((x & 0xF0F0) >> 4) | ((x & 0x0F0F) << 4);
    (((x & 0xFF00) >> 8) | ((x & 0x00FF) << 8)) >> 1
}

/// O(1) equivalent of `revfd[d]` for `1 <= d <= 32768`: `((d - fd[i]) << 5) | i`.
#[inline]
pub const fn revfd(d: usize) -> u32 {
    if d <= 4 {
        if d == 0 { 0 } else { (d - 1) as u32 }
    } else {
        let dm1 = (d - 1) as u32;
        let k = 31 - dm1.leading_zeros();
        let idx = (k << 1) + ((dm1 >> (k - 1)) & 1);
        let extra = dm1 & ((1u32 << (k - 1)) - 1);
        (extra << 5) | idx
    }
}

/// Exact integer equivalent of `Math.ceil(Math.max(8, Math.min(13, Math.log(len))) * 1.5)`.
#[inline]
pub const fn default_plvl(len: usize) -> usize {
    if len <= 2980 {
        12
    } else if len <= 5806 {
        13
    } else if len <= 11308 {
        14
    } else if len <= 22026 {
        15
    } else if len <= 42901 {
        16
    } else if len <= 83561 {
        17
    } else if len <= 162754 {
        18
    } else if len <= 317003 {
        19
    } else {
        20
    }
}

const fn make_fl() -> [u16; 31] {
    let mut b = [0u16; 31];
    let mut start: u32 = 2;
    let mut i = 0;
    while i < 31 {
        let step = if i == 0 { 1u32 } else { 1u32 << FLEB[i - 1] };
        start += step;
        b[i] = start as u16;
        i += 1;
    }
    b[28] = 258;
    b
}

const fn make_revfl() -> [u16; 259] {
    let mut b = [0u16; 31];
    let mut start: u32 = 2;
    let mut i = 0;
    while i < 31 {
        let step = if i == 0 { 1u32 } else { 1u32 << FLEB[i - 1] };
        start += step;
        b[i] = start as u16;
        i += 1;
    }
    let mut r = [0u16; 259];
    let mut idx = 1;
    while idx < 28 {
        let mut j = b[idx] as usize;
        let end = b[idx + 1] as usize;
        while j < end {
            r[j] = (((j - b[idx] as usize) << 5) | idx) as u16;
            j += 1;
        }
        idx += 1;
    }
    r[258] = 28;
    r
}

const fn make_fd() -> [u16; 31] {
    let mut b = [0u16; 31];
    let mut start: u32 = 0;
    let mut i = 0;
    while i < 31 {
        let step = if i == 0 { 1u32 } else { 1u32 << FDEB[i - 1] };
        start += step;
        b[i] = start as u16;
        i += 1;
    }
    b
}

const fn make_flt() -> [u8; 288] {
    let mut t = [0u8; 288];
    let mut i = 0;
    while i < 144 {
        t[i] = 8;
        i += 1;
    }
    while i < 256 {
        t[i] = 9;
        i += 1;
    }
    while i < 280 {
        t[i] = 7;
        i += 1;
    }
    while i < 288 {
        t[i] = 8;
        i += 1;
    }
    t
}

const fn make_fixed_forward_288() -> [u16; 288] {
    let cd = make_flt();
    let mut l = [0u16; 9];
    let mut i = 0;
    while i < 288 {
        l[(cd[i] - 1) as usize] += 1;
        i += 1;
    }
    let mut le = [0u16; 9];
    i = 1;
    while i < 9 {
        le[i] = (le[i - 1] + l[i - 1]) << 1;
        i += 1;
    }
    let mut co = [0u16; 288];
    i = 0;
    while i < 288 {
        let bits = cd[i] as usize;
        let code = le[bits - 1] as u32;
        le[bits - 1] = le[bits - 1].wrapping_add(1);
        co[i] = (rev15(code) >> (15 - bits)) as u16;
        i += 1;
    }
    co
}

const fn make_fixed_reverse_512() -> [u16; 512] {
    let cd = make_flt();
    let mut l = [0u16; 9];
    let mut i = 0;
    while i < 288 {
        l[(cd[i] - 1) as usize] += 1;
        i += 1;
    }
    let mut le = [0u16; 9];
    i = 1;
    while i < 9 {
        le[i] = (le[i - 1] + l[i - 1]) << 1;
        i += 1;
    }
    let mut co = [0u16; 512];
    let rvb = 15 - 9;
    i = 0;
    while i < 288 {
        let bits = cd[i] as usize;
        let sv = ((i as u16) << 4) | (bits as u16);
        let r1 = 9 - bits;
        let mut v = (le[bits - 1] as u32) << r1;
        le[bits - 1] = le[bits - 1].wrapping_add(1);
        let m = v | ((1u32 << r1) - 1);
        while v <= m {
            co[(rev15(v) >> rvb) as usize] = sv;
            v += 1;
        }
        i += 1;
    }
    co
}

const fn make_fixed_forward_32() -> [u16; 32] {
    let mut co = [0u16; 32];
    let mut i = 0;
    while i < 32 {
        co[i] = (rev15(i as u32) >> 10) as u16;
        i += 1;
    }
    co
}

const fn make_fixed_reverse_32() -> [u16; 32] {
    let mut co = [0u16; 32];
    let mut i = 0;
    while i < 32 {
        let sv = ((i as u16) << 4) | 5;
        co[(rev15(i as u32) >> 10) as usize] = sv;
        i += 1;
    }
    co
}

const fn make_crct() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut i = 0u32;
    while i < 256 {
        let mut c = i;
        let mut k = 8;
        while k > 0 {
            c = if (c & 1) != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            k -= 1;
        }
        t[i as usize] = c;
        i += 1;
    }
    t
}
