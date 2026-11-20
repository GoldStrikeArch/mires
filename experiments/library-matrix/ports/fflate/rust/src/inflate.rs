//! DEFLATE inflate state machine (`inflt`, `inflate_sync`), matching fflate 0.8.3 bit-for-bit
//! including zero-padded OOB bit reads, overlapping RLE copies, custom dictionary back-references,
//! and pre-allocated output buffer sizing (`outSize`).

use alloc::vec;
use alloc::vec::Vec;

use crate::deflate::shft;
use crate::framing::FlateError;
use crate::huffman::h_map;
use crate::tables::{CLIM, FD, FDEB, FDRM, FL, FLEB, FLRM};

/// Read bits starting at bit offset `p` masked by `m`, treating out-of-bounds reads as 0.
#[inline]
fn bits(d: &[u8], p: usize, m: u32) -> u32 {
    let o = p >> 3;
    let b0 = if o < d.len() { d[o] as u32 } else { 0 };
    let b1 = if o + 1 < d.len() { (d[o + 1] as u32) << 8 } else { 0 };
    ((b0 | b1) >> (p & 7)) & m
}

#[inline]
fn max_byte(slice: &[u8]) -> usize {
    let mut m: u8 = 0;
    let mut i = 0;
    while i < slice.len() {
        if slice[i] > m {
            m = slice[i];
        }
        i += 1;
    }
    m as usize
}

/// Either the static fixed Huffman reverse table or an owned dynamic table.
pub enum HuffMap {
    FixedLit,
    FixedDist,
    Dynamic(Vec<u16>),
}

impl HuffMap {
    #[inline]
    fn as_slice(&self) -> &[u16] {
        match self {
            HuffMap::FixedLit => &FLRM,
            HuffMap::FixedDist => &FDRM,
            HuffMap::Dynamic(v) => v.as_slice(),
        }
    }
}

/// Persistent inflate state across streaming chunks (fflate `InflateState`).
pub struct InflateState {
    /// Mode: `0` = non-final stream chunk, `1` = final stream chunk, `2` = one-shot sync (`st.i`).
    pub i: u8,
    /// Last-block (`BFINAL`) flag (`st.f`).
    pub f: u8,
    /// Bit position inside current input chunk (`st.p`).
    pub p: usize,
    /// Output byte write position (`st.b`).
    pub b: usize,
    /// Active literal/length reverse Huffman table (`st.l`).
    pub l: Option<HuffMap>,
    /// Active distance reverse Huffman table (`st.d`).
    pub d: Option<HuffMap>,
    /// Max bits of literal/length table (`st.m`).
    pub m: usize,
    /// Max bits of distance table (`st.n`).
    pub n: usize,
}

impl InflateState {
    pub fn oneshot() -> Self {
        Self {
            i: 2,
            f: 0,
            p: 0,
            b: 0,
            l: None,
            d: None,
            m: 0,
            n: 0,
        }
    }

    pub fn streaming(initial_b: usize) -> Self {
        Self {
            i: 0,
            f: 0,
            p: 0,
            b: initial_b,
            l: None,
            d: None,
            m: 0,
            n: 0,
        }
    }
}

/// Ensure `buf` can hold at least `needed` bytes by doubling or jumping to `needed`.
#[inline]
fn cbuf(buf: &mut Vec<u8>, needed: usize) {
    let bl = buf.len();
    if needed > bl {
        let new_len = if bl * 2 > needed { bl * 2 } else { needed };
        buf.resize(new_len, 0);
    }
}

/// Core DEFLATE decompressor (fflate `inflt`).
///
/// When `buf` is `None`, allocates a fresh growable buffer (`noBuf = true`) and returns a slice
/// trimmed to `bt`. When `buf` is `Some(v)`:
/// - If `st.i == 2` (one-shot with `out` / `gunzipSync`), `resize` is `false`: stored-block
///   overflow returns `FlateError::RangeError`, Huffman overflow silently truncates, and the
///   returned `Vec<u8>` is truncated to `min(bt, buf.len())` (or untouched on empty-input early
///   return, matching fflate's `return buf || new u8(0)`).
/// - If `st.i != 2` (streaming `Inflate`), `resize` is `true` and `buf` grows as needed, returning
///   the buffer sliced to `st.b`.
pub fn inflt(
    dat: &[u8],
    st: &mut InflateState,
    buf_opt: Option<Vec<u8>>,
    dict: Option<&[u8]>,
) -> Result<Vec<u8>, FlateError> {
    let sl = dat.len();
    let dl = match dict {
        Some(d) => d.len(),
        None => 0,
    };
    if sl == 0 || (st.f != 0 && st.l.is_none()) {
        return Ok(match buf_opt {
            Some(b) => b,
            None => Vec::new(),
        });
    }

    let no_buf = buf_opt.is_none();
    let resize = no_buf || st.i != 2;
    let no_st = st.i != 0;

    let mut buf = match buf_opt {
        Some(b) => b,
        None => vec![0u8; sl * 3],
    };

    let mut is_final = st.f;
    let mut pos = st.p;
    let mut bt = st.b;
    let mut lm = st.l.take();
    let mut dm = st.d.take();
    let mut lbt = st.m;
    let mut dbt = st.n;

    let tbts = sl * 8;

    loop {
        if lm.is_none() {
            is_final = bits(dat, pos, 1) as u8;
            let btype = bits(dat, pos + 1, 3);
            pos += 3;
            if btype == 0 {
                let s = shft(pos) + 4;
                let b0 = if s - 4 < sl { dat[s - 4] as usize } else { 0 };
                let b1 = if s - 3 < sl { (dat[s - 3] as usize) << 8 } else { 0 };
                let l = b0 | b1;
                let t = s + l;
                if t > sl {
                    if no_st {
                        return Err(FlateError::Code(0));
                    }
                    break;
                }
                if resize {
                    cbuf(&mut buf, bt + l);
                } else if bt + l > buf.len() {
                    return Err(FlateError::RangeError);
                }
                buf[bt..bt + l].copy_from_slice(&dat[s..t]);
                bt += l;
                pos = t * 8;
                st.b = bt;
                st.p = pos;
                st.f = is_final;
                if is_final != 0 {
                    break;
                }
                continue;
            } else if btype == 1 {
                lm = Some(HuffMap::FixedLit);
                dm = Some(HuffMap::FixedDist);
                lbt = 9;
                dbt = 5;
            } else if btype == 2 {
                let h_lit = (bits(dat, pos, 31) as usize) + 257;
                let h_dist = (bits(dat, pos + 5, 31) as usize) + 1;
                let hc_len = (bits(dat, pos + 10, 15) as usize) + 4;
                let tl = h_lit + h_dist;
                pos += 14;

                let mut ldt = vec![0u8; tl];
                let mut clt = [0u8; 19];
                let mut i = 0;
                while i < hc_len {
                    clt[CLIM[i]] = bits(dat, pos + i * 3, 7) as u8;
                    i += 1;
                }
                pos += hc_len * 3;
                let clb = max_byte(&clt);
                let clbmsk = (1u32 << clb) - 1;
                let clm = h_map(&clt, clb, true);

                let mut idx = 0;
                while idx < tl {
                    let r = clm[bits(dat, pos, clbmsk) as usize] as usize;
                    pos += r & 15;
                    let s = r >> 4;
                    if s < 16 {
                        ldt[idx] = s as u8;
                        idx += 1;
                    } else {
                        let mut c: u8 = 0;
                        let mut n: usize = 0;
                        if s == 16 {
                            n = 3 + (bits(dat, pos, 3) as usize);
                            pos += 2;
                            c = if idx > 0 { ldt[idx - 1] } else { 0 };
                        } else if s == 17 {
                            n = 3 + (bits(dat, pos, 7) as usize);
                            pos += 3;
                        } else if s == 18 {
                            n = 11 + (bits(dat, pos, 127) as usize);
                            pos += 7;
                        }
                        while n > 0 {
                            if idx < tl {
                                ldt[idx] = c;
                            }
                            idx += 1;
                            n -= 1;
                        }
                    }
                }
                let lt = &ldt[..h_lit];
                let dt_sl = &ldt[h_lit..];
                lbt = max_byte(lt);
                dbt = max_byte(dt_sl);
                lm = Some(HuffMap::Dynamic(h_map(lt, lbt, true)));
                dm = Some(HuffMap::Dynamic(h_map(dt_sl, dbt, true)));
            } else {
                return Err(FlateError::Code(1));
            }
            if pos > tbts {
                if no_st {
                    return Err(FlateError::Code(0));
                }
                break;
            }
        }

        if resize {
            cbuf(&mut buf, bt + 131072);
        }
        let lms = (1u32 << lbt) - 1;
        let dms = (1u32 << dbt) - 1;
        let mut lpos = pos;
        let mut block_done = false;

        macro_rules! read_bits16 {
            ($p:expr) => {{
                let p_ = $p;
                let o_ = p_ >> 3;
                let w_ = if o_ + 2 < sl {
                    (dat[o_] as u32) | ((dat[o_ + 1] as u32) << 8) | ((dat[o_ + 2] as u32) << 16)
                } else {
                    let b0_ = if o_ < sl { dat[o_] as u32 } else { 0 };
                    let b1_ = if o_ + 1 < sl { (dat[o_ + 1] as u32) << 8 } else { 0 };
                    b0_ | b1_
                };
                w_ >> (p_ & 7)
            }};
        }

        // Bind slice handles outside the inner loop to avoid per-symbol Vec derefs / calls.
        {
            let lm_sl = match lm.as_ref() {
                Some(m) => m.as_slice(),
                None => &FLRM,
            };
            let dm_sl = match dm.as_ref() {
                Some(m) => m.as_slice(),
                None => &FDRM,
            };
            let mut buf_len = buf.len();
            let mut out_sl: &mut [u8] = &mut buf;

            loop {
                lpos = pos;
                let c = lm_sl[(read_bits16!(pos) & lms) as usize] as usize;
                let sym = c >> 4;
                pos += c & 15;
                if pos > tbts {
                    if no_st {
                        return Err(FlateError::Code(0));
                    }
                    break;
                }
                if c == 0 {
                    return Err(FlateError::Code(2));
                }
                if sym < 256 {
                    if resize && bt >= buf_len {
                        cbuf(&mut buf, bt + 131072);
                        buf_len = buf.len();
                        out_sl = &mut buf;
                    }
                    if bt < buf_len {
                        out_sl[bt] = sym as u8;
                    }
                    bt += 1;
                } else if sym == 256 {
                    lpos = pos;
                    block_done = true;
                    break;
                } else {
                    let add = if sym > 264 {
                        let idx = sym - 257;
                        let b = FLEB[idx] as usize;
                        let extra = (read_bits16!(pos) & ((1u32 << b) - 1)) as usize;
                        pos += b;
                        extra + (FL[idx] as usize)
                    } else {
                        sym - 254
                    };

                    let d_entry = dm_sl[(read_bits16!(pos) & dms) as usize] as usize;
                    let dsym = d_entry >> 4;
                    if d_entry == 0 {
                        return Err(FlateError::Code(3));
                    }
                    pos += d_entry & 15;
                    let d_oob = dsym >= FD.len();
                    let mut dt = if d_oob { 0 } else { FD[dsym] as usize };
                    if dsym > 3 {
                        let b = FDEB[dsym] as usize;
                        dt += (read_bits16!(pos) & ((1u32 << b) - 1)) as usize;
                        pos += b;
                    }
                    if pos > tbts {
                        if no_st {
                            return Err(FlateError::Code(0));
                        }
                        break;
                    }
                    if resize && bt + 131072 > buf_len {
                        cbuf(&mut buf, bt + 131072);
                        buf_len = buf.len();
                        out_sl = &mut buf;
                    }
                    let end = bt + add;

                    if d_oob {
                        while bt < end {
                            if bt < buf_len {
                                out_sl[bt] = 0;
                            }
                            bt += 1;
                        }
                        continue;
                    }

                    if bt < dt {
                        let shift = (dl as isize) - (dt as isize);
                        let dend = if dt < end { dt } else { end };
                        if shift + (bt as isize) < 0 {
                            return Err(FlateError::Code(3));
                        }
                        if let Some(d_bytes) = dict {
                            while bt < dend {
                                let v = d_bytes[(shift + (bt as isize)) as usize];
                                if bt < buf_len {
                                    out_sl[bt] = v;
                                }
                                bt += 1;
                            }
                        }
                    }

                    if end <= buf_len {
                        while bt < end {
                            out_sl[bt] = out_sl[bt - dt];
                            bt += 1;
                        }
                    } else {
                        while bt < end {
                            if bt < buf_len {
                                let src = bt - dt;
                                out_sl[bt] = if src < buf_len { out_sl[src] } else { 0 };
                            }
                            bt += 1;
                        }
                    }
                }
            }
        }

        if block_done {
            lm = None;
        }
        st.p = lpos;
        st.b = bt;
        st.f = is_final;
        if lm.is_some() {
            st.l = lm;
            st.d = dm;
            st.m = lbt;
            st.n = dbt;
            break;
        }
        st.l = None;
        if is_final != 0 {
            break;
        }
    }

    let out_len = if bt < buf.len() { bt } else { buf.len() };
    buf.truncate(out_len);
    Ok(buf)
}

/// Decompression options shared by one-shot and streaming decompressors.
pub struct DecompressOptions {
    pub dictionary: Option<Vec<u8>>,
    pub out_size: Option<usize>,
}

impl DecompressOptions {
    pub fn empty() -> Self {
        Self {
            dictionary: None,
            out_size: None,
        }
    }
}

/// One-shot raw DEFLATE decompression (fflate `inflateSync`).
pub fn inflate_sync(data: &[u8], opts: &DecompressOptions) -> Result<Vec<u8>, FlateError> {
    let mut st = InflateState::oneshot();
    let buf = opts.out_size.map(|n| vec![0u8; n]);
    inflt(data, &mut st, buf, opts.dictionary.as_deref())
}
