//! Checksums (CRC-32, Adler-32), GZIP/Zlib framing, format auto-detection (`decompress_sync`),
//! and `FlateError` codes, matching fflate 0.8.3 bit-for-bit.

use alloc::vec;
use alloc::vec::Vec;

use crate::deflate::{dopt, CompressOptions};
use crate::inflate::{inflt, inflate_sync, DecompressOptions, InflateState};
use crate::tables::CRCT;

/// Error returned by fflate operations: either a `FlateErrorCode` (`0..=6`) or a JS `RangeError`
/// (e.g., stored-block overflow against a fixed `outSize` buffer).
#[derive(Clone, Copy)]
pub enum FlateError {
    Code(u8),
    RangeError,
}

/// Write a 32-bit unsigned value little-endian into zeroed storage (fflate `wbytes`).
#[inline]
pub fn wbytes(d: &mut [u8], mut b: usize, mut v: u32) {
    while v != 0 {
        d[b] = (v & 0xFF) as u8;
        v >>= 8;
        b += 1;
    }
}

/// Streaming / one-shot IEEE CRC-32 (fflate `crc`).
pub struct Crc32 {
    c: u32,
}

impl Crc32 {
    pub fn new() -> Self {
        Self { c: 0xFFFF_FFFF }
    }

    pub fn push(&mut self, d: &[u8]) {
        let mut cr = self.c;
        let len = d.len();
        let mut i = 0;
        while i < len {
            cr = CRCT[((cr & 0xFF) ^ (d[i] as u32)) as usize] ^ (cr >> 8);
            i += 1;
        }
        self.c = cr;
    }

    #[inline]
    pub fn digest(&self) -> u32 {
        !self.c
    }
}

/// Streaming / one-shot Adler-32 (fflate `adler`).
pub struct Adler32 {
    a: u32,
    b: u32,
}

impl Adler32 {
    pub fn new() -> Self {
        Self { a: 1, b: 0 }
    }

    pub fn push(&mut self, d: &[u8]) {
        let mut n = self.a;
        let mut m = self.b;
        let l = d.len();
        let mut i = 0;
        while i != l {
            let e = if i + 2655 < l { i + 2655 } else { l };
            while i < e {
                n += d[i] as u32;
                m += n;
                i += 1;
            }
            n = (n & 65535) + 15 * (n >> 16);
            m = (m & 65535) + 15 * (m >> 16);
        }
        self.a = n;
        self.b = m;
    }

    /// Returns byte-swapped 32-bit value matching fflate `adler().d()` so `wbytes` writes big-endian.
    #[inline]
    pub fn digest_for_wbytes(&mut self) -> u32 {
        self.a %= 65521;
        self.b %= 65521;
        ((self.a & 255) << 24)
            | ((self.a & 0xFF00) << 8)
            | ((self.b & 255) << 8)
            | (self.b >> 8)
    }
}

/// GZIP header length (fflate `gzhl`). Empty filename is falsy -> 10 bytes.
#[inline]
pub fn gzhl(o: &CompressOptions) -> usize {
    match o.filename.as_ref() {
        Some(f) if !f.is_empty() => 10 + f.len() + 1,
        _ => 10,
    }
}

/// Write GZIP header (fflate `gzh`).
pub fn gzh(c: &mut [u8], o: &CompressOptions) {
    c[0] = 31;
    c[1] = 139;
    c[2] = 8;
    c[8] = match o.level {
        Some(lvl) if lvl < 2 => 4,
        Some(9) => 2,
        _ => 0,
    };
    c[9] = 3;
    if let Some(mtime) = o.mtime {
        if mtime != 0.0 {
            let secs = (mtime / 1000.0) as u32;
            wbytes(c, 4, secs);
        }
    }
    if let Some(ref fn_bytes) = o.filename {
        if !fn_bytes.is_empty() {
            c[3] = 8;
            let len = fn_bytes.len();
            c[10..10 + len].copy_from_slice(fn_bytes);
            c[10 + len] = 0;
        }
    }
}

/// Parse GZIP header and return offset to DEFLATE payload (fflate `gzs`).
pub fn gzs(d: &[u8]) -> Result<usize, FlateError> {
    let b0 = if !d.is_empty() { d[0] } else { 0 };
    let b1 = if d.len() > 1 { d[1] } else { 0 };
    let b2 = if d.len() > 2 { d[2] } else { 0 };
    if b0 != 31 || b1 != 139 || b2 != 8 {
        return Err(FlateError::Code(6));
    }
    let flg = if d.len() > 3 { d[3] as usize } else { 0 };
    let mut st: usize = 10;
    if (flg & 4) != 0 {
        let x0 = if d.len() > 10 { d[10] as usize } else { 0 };
        let x1 = if d.len() > 11 { (d[11] as usize) << 8 } else { 0 };
        st += (x0 | x1) + 2;
    }
    let mut zs = ((flg >> 3) & 1) + ((flg >> 4) & 1);
    while zs > 0 {
        let b = if st < d.len() { d[st] } else { 0 };
        st += 1;
        if b == 0 {
            zs -= 1;
        }
    }
    Ok(st + (flg & 2))
}

/// Read GZIP 32-bit ISIZE trailer (fflate `gzl`).
#[inline]
pub fn gzl(d: &[u8]) -> usize {
    let l = d.len();
    ((d[l - 4] as u32)
        | ((d[l - 3] as u32) << 8)
        | ((d[l - 2] as u32) << 16)
        | ((d[l - 1] as u32) << 24)) as usize
}

/// One-shot GZIP compression (fflate `gzipSync`).
pub fn gzip_sync(data: &[u8], opts: &CompressOptions) -> Vec<u8> {
    let mut c = Crc32::new();
    let l = data.len();
    c.push(data);
    let mut d = dopt(data, opts, gzhl(opts), 8, None);
    let s = d.len();
    gzh(&mut d, opts);
    wbytes(&mut d, s - 8, c.digest());
    wbytes(&mut d, s - 4, l as u32);
    d
}

/// One-shot GZIP decompression (fflate `gunzipSync`).
pub fn gunzip_sync(data: &[u8], opts: &DecompressOptions) -> Result<Vec<u8>, FlateError> {
    let st = gzs(data)?;
    if st + 8 > data.len() {
        return Err(FlateError::Code(6));
    }
    let out_len = match opts.out_size {
        Some(n) => n,
        None => gzl(data),
    };
    let mut state = InflateState::oneshot();
    inflt(
        &data[st..data.len() - 8],
        &mut state,
        Some(vec![0u8; out_len]),
        opts.dictionary.as_deref(),
    )
}

/// Write Zlib header (fflate `zlh`).
pub fn zlh(c: &mut [u8], o: &CompressOptions) {
    let fl: u8 = match o.level {
        Some(0) => 0,
        Some(lvl) if lvl < 6 => 1,
        Some(9) => 3,
        _ => 2,
    };
    c[0] = 120;
    let has_dict = o.dictionary.is_some();
    c[1] = (fl << 6) | if has_dict { 32 } else { 0 };
    let chk = (((c[0] as u32) << 8) | (c[1] as u32)) % 31;
    c[1] |= (31 - chk) as u8;
    if let Some(ref dict) = o.dictionary {
        let mut h = Adler32::new();
        h.push(dict);
        wbytes(c, 2, h.digest_for_wbytes());
    }
}

/// Parse Zlib header and return offset to DEFLATE payload (fflate `zls`).
pub fn zls(d: &[u8], has_dict: bool) -> Result<usize, FlateError> {
    let b0 = if !d.is_empty() { d[0] as u32 } else { 0 };
    let b1 = if d.len() > 1 { d[1] as u32 } else { 0 };
    if (b0 & 15) != 8 || (b0 >> 4) > 7 || (((b0 << 8) | b1) % 31) != 0 {
        return Err(FlateError::Code(6));
    }
    let fdict = ((b1 >> 5) & 1) != 0;
    if fdict != has_dict {
        return Err(FlateError::Code(6));
    }
    Ok((((b1 >> 3) & 4) + 2) as usize)
}

/// One-shot Zlib compression (fflate `zlibSync`).
pub fn zlib_sync(data: &[u8], opts: &CompressOptions) -> Vec<u8> {
    let mut a = Adler32::new();
    a.push(data);
    let pre = if opts.dictionary.is_some() { 6 } else { 2 };
    let mut d = dopt(data, opts, pre, 4, None);
    let len = d.len();
    zlh(&mut d, opts);
    wbytes(&mut d, len - 4, a.digest_for_wbytes());
    d
}

/// One-shot Zlib decompression (fflate `unzlibSync`).
pub fn unzlib_sync(data: &[u8], opts: &DecompressOptions) -> Result<Vec<u8>, FlateError> {
    let start = zls(data, opts.dictionary.is_some())?;
    let end = if data.len() >= 4 { data.len() - 4 } else { 0 };
    let payload = if start < end { &data[start..end] } else { &[] };
    let mut state = InflateState::oneshot();
    let buf = opts.out_size.map(|n| vec![0u8; n]);
    inflt(payload, &mut state, buf, opts.dictionary.as_deref())
}

/// Auto-detect GZIP, Zlib, or raw DEFLATE format (shared by `decompressSync` and `Decompress`).
#[inline]
pub fn detect_format(d: &[u8]) -> u8 {
    let b0 = if !d.is_empty() { d[0] as u32 } else { 0 };
    let b1 = if d.len() > 1 { d[1] as u32 } else { 0 };
    let b2 = if d.len() > 2 { d[2] as u32 } else { 0 };
    if b0 == 31 && b1 == 139 && b2 == 8 {
        0 // gzip
    } else if (b0 & 15) != 8 || (b0 >> 4) > 7 || (((b0 << 8) | b1) % 31) != 0 {
        1 // raw deflate
    } else {
        2 // zlib
    }
}

/// One-shot auto-detecting decompression (fflate `decompressSync`).
pub fn decompress_sync(data: &[u8], opts: &DecompressOptions) -> Result<Vec<u8>, FlateError> {
    match detect_format(data) {
        0 => gunzip_sync(data, opts),
        1 => inflate_sync(data, opts),
        _ => unzlib_sync(data, opts),
    }
}
