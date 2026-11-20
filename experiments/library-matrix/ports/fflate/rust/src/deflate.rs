//! DEFLATE block writing (`wbits`, `wbits16`, `wfblk`, `wblk`) and LZ77 sliding-window
//! compressor (`dflt`, `dopt`, `deflate_sync`), matching fflate 0.8.3 bit-for-bit.

use alloc::vec;
use alloc::vec::Vec;

use crate::huffman::{clen, h_map, h_tree, lc};
use crate::tables::{default_plvl, revfd, CLIM, DEO, FDEB, FDM, FDT, FLEB, FLM, FLT, REVFL};

/// Round bit count up to whole bytes (fflate `shft`).
#[inline]
pub const fn shft(p: usize) -> usize {
    (p + 7) >> 3
}

/// Write up to 9 bits starting at bit offset `p` (fflate `wbits`).
#[inline]
fn wbits(d: &mut [u8], p: usize, v: u32) {
    let shifted = v << (p & 7);
    let o = p >> 3;
    d[o] |= (shifted & 0xFF) as u8;
    d[o + 1] |= ((shifted >> 8) & 0xFF) as u8;
}

/// Write an uncompressed stored block (BTYPE=00 already zeroed; fflate `wfblk`).
pub fn wfblk(out: &mut [u8], pos: usize, dat: &[u8]) -> usize {
    let s = dat.len();
    let o = shft(pos + 2);
    let lo = (s & 0xFF) as u8;
    let hi = ((s >> 8) & 0xFF) as u8;
    out[o] = lo;
    out[o + 1] = hi;
    out[o + 2] = lo ^ 0xFF;
    out[o + 3] = hi ^ 0xFF;
    out[o + 4..o + 4 + s].copy_from_slice(dat);
    (o + 4 + s) * 8
}

/// Emit one DEFLATE block, choosing between stored, fixed Huffman, and dynamic Huffman using
/// fflate's exact cost formulas and tie-breaks (fflate `wblk`).
#[allow(clippy::too_many_arguments)]
fn wblk(
    dat: &[u8],
    out: &mut [u8],
    is_final: u32,
    syms: &[u32],
    lf: &mut [u16; 288],
    df: &[u16; 32],
    eb: usize,
    li: usize,
    bs: usize,
    bl: usize,
    mut p: usize,
) -> usize {
    wbits(out, p, is_final);
    p += 1;
    lf[256] = lf[256].wrapping_add(1);

    let (dlt, mlb) = h_tree(lf, 15);
    let (ddt, mdb) = h_tree(df, 15);
    let (lclt, nlc) = lc(&dlt);
    let (lcdt, ndc) = lc(&ddt);

    let mut lcfreq = [0u16; 19];
    for &entry in &lclt {
        lcfreq[(entry & 31) as usize] += 1;
    }
    for &entry in &lcdt {
        lcfreq[(entry & 31) as usize] += 1;
    }
    let (lct, mlcb) = h_tree(&lcfreq, 7);

    let mut nlcc: usize = 19;
    while nlcc > 4 {
        let sym = CLIM[nlcc - 1];
        if sym < lct.len() && lct[sym] != 0 {
            break;
        }
        nlcc -= 1;
    }

    let flen = (bl + 5) << 3;
    let ftlen = clen(lf, &FLT) + clen(df, &FDT) + eb;
    let dtlen = clen(lf, &dlt)
        + clen(df, &ddt)
        + eb
        + 14
        + 3 * nlcc
        + clen(&lcfreq, &lct)
        + 2 * (lcfreq[16] as usize)
        + 3 * (lcfreq[17] as usize)
        + 7 * (lcfreq[18] as usize);

    // Stored block wins ties against both fixed and dynamic.
    if flen <= ftlen && flen <= dtlen {
        return wfblk(out, p, &dat[bs..bs + bl]);
    }

    let use_dynamic = dtlen < ftlen;
    wbits(out, p, 1 + if use_dynamic { 1 } else { 0 });
    p += 2;

    if use_dynamic {
        let lm = h_map(&dlt, mlb, false);
        let dm = h_map(&ddt, mdb, false);
        let llm = h_map(&lct, mlcb, false);
        wbits(out, p, (nlc - 257) as u32);
        wbits(out, p + 5, (ndc - 1) as u32);
        wbits(out, p + 10, (nlcc - 4) as u32);
        p += 14;
        let mut i = 0;
        while i < nlcc {
            let sym = CLIM[i];
            let val = if sym < lct.len() { lct[sym] as u32 } else { 0 };
            wbits(out, p + 3 * i, val);
            i += 1;
        }
        p += 3 * nlcc;

        // Emit lclt then lcdt.
        let mut pass = 0;
        while pass < 2 {
            let clct: &[u16] = if pass == 0 { &lclt } else { &lcdt };
            let mut j = 0;
            while j < clct.len() {
                let entry = clct[j] as u32;
                let len = (entry & 31) as usize;
                wbits(out, p, llm[len] as u32);
                p += lct[len] as usize;
                if len > 15 {
                    wbits(out, p, (entry >> 5) & 127);
                    p += (entry >> 12) as usize;
                }
                j += 1;
            }
            pass += 1;
        }
        p = emit_symbols(out, p, syms, li, &lm, &dlt, &dm, &ddt);
    } else {
        p = emit_symbols(out, p, syms, li, &FLM, &FLT, &FDM, &FDT);
    }
    p
}

#[inline]
#[allow(clippy::too_many_arguments)]
fn emit_symbols(
    out: &mut [u8],
    mut p: usize,
    syms: &[u32],
    li: usize,
    lm: &[u16],
    ll: &[u8],
    dm: &[u16],
    dl: &[u8],
) -> usize {
    macro_rules! write_bits16 {
        ($p:expr, $v:expr) => {{
            let p_ = $p;
            let shifted_ = ($v) << (p_ & 7);
            let o_ = p_ >> 3;
            out[o_] |= (shifted_ & 0xFF) as u8;
            out[o_ + 1] |= ((shifted_ >> 8) & 0xFF) as u8;
            out[o_ + 2] |= ((shifted_ >> 16) & 0xFF) as u8;
        }};
    }
    let mut i = 0;
    while i < li {
        let sym = syms[i];
        if sym > 255 {
            let len = ((sym >> 18) & 31) as usize;
            let lsym = len + 257;
            write_bits16!(p, lm[lsym] as u32);
            p += ll[lsym] as usize;
            if len > 7 {
                wbits(out, p, (sym >> 23) & 31);
                p += FLEB[len] as usize;
            }
            let dst = (sym & 31) as usize;
            write_bits16!(p, dm[dst] as u32);
            p += dl[dst] as usize;
            if dst > 3 {
                write_bits16!(p, (sym >> 5) & 8191);
                p += FDEB[dst] as usize;
            }
        } else {
            let s = sym as usize;
            write_bits16!(p, lm[s] as u32);
            p += ll[s] as usize;
        }
        i += 1;
    }
    write_bits16!(p, lm[256] as u32);
    p + ll[256] as usize
}

/// Compression options shared by one-shot and streaming compressors.
pub struct CompressOptions {
    pub level: Option<u8>,
    pub mem: Option<u8>,
    pub dictionary: Option<Vec<u8>>,
    pub mtime: Option<f64>,
    pub filename: Option<Vec<u8>>,
}

impl CompressOptions {
    pub fn empty() -> Self {
        Self {
            level: None,
            mem: None,
            dictionary: None,
            mtime: None,
            filename: None,
        }
    }
}

/// Persistent compressor state across streaming chunks (fflate `DeflateState`).
pub struct DeflateState {
    /// Final-chunk flag (`st.l`: 0 or 1).
    pub l: u32,
    /// Trailing partial bit count and byte (`st.r = (pos & 7) | (byte << 3)`).
    pub r: usize,
    /// Hash head table (`st.h`), lazily allocated on first non-level-0 call.
    pub h: Option<Vec<u16>>,
    /// Hash chain prev table (`st.p`), lazily allocated on first non-level-0 call.
    pub p: Option<Vec<u16>>,
    /// Current scan index (`st.i`).
    pub i: usize,
    /// Wait index (`st.w`): data before `w` is lookback / already matched.
    pub w: usize,
    /// Active length inside `dat` (`st.z`; `0` means `dat.len()`).
    pub z: usize,
}

impl DeflateState {
    pub fn oneshot() -> Self {
        Self {
            l: 1,
            r: 0,
            h: None,
            p: None,
            i: 0,
            w: 0,
            z: 0,
        }
    }

    pub fn streaming() -> Self {
        Self {
            l: 0,
            r: 0,
            h: None,
            p: None,
            i: 32768,
            w: 32768,
            z: 32768,
        }
    }
}

/// Core DEFLATE compressor (fflate `dflt`).
pub fn dflt(
    dat: &[u8],
    lvl: u8,
    plvl: usize,
    pre: usize,
    post: usize,
    st: &mut DeflateState,
) -> Vec<u8> {
    let s = if st.z != 0 { st.z } else { dat.len() };
    let extra_blocks = 1 + if s == 0 { 0 } else { (s + 6999) / 7000 };
    // +2 slack bytes inside `w` so `wbits16` never writes out of bounds on final EOB.
    let w_len = s + 5 * extra_blocks + 2;
    let mut o = vec![0u8; pre + w_len + post];
    let lst = st.l;
    let mut pos = st.r & 7;

    if lvl != 0 {
        let w: &mut [u8] = &mut o[pre..pre + w_len];
        if pos != 0 {
            w[0] = (st.r >> 3) as u8;
        }
        let opt = DEO[(lvl - 1) as usize];
        let n = (opt >> 13) as usize;
        let c = (opt & 8191) as usize;
        let msk = (1usize << plvl) - 1;
        let bs1 = (plvl + 2) / 3;
        let bs2 = 2 * bs1;

        let mut prev_vec = match st.p.take() {
            Some(v) => v,
            None => vec![0u16; 32768],
        };
        let mut head_vec = match st.h.take() {
            Some(v) => v,
            None => vec![0u16; msk + 1],
        };
        let prev: &mut [u16] = &mut prev_vec;
        let head: &mut [u16] = &mut head_vec;

        let mut syms_vec = vec![0u32; 25000];
        let syms: &mut [u32] = &mut syms_vec;
        let mut lf = [0u16; 288];
        let mut df = [0u16; 32];

        let mut lc_cnt: usize = 0;
        let mut eb: usize = 0;
        let mut i: usize = st.i;
        let mut li: usize = 0;
        let mut wi: usize = st.w;
        let mut bs: usize = 0;

        while i + 2 < s {
            let hv = ((dat[i] as usize)
                ^ ((dat[i + 1] as usize) << bs1)
                ^ ((dat[i + 2] as usize) << bs2))
                & msk;
            let mut imod = i & 32767;
            let mut pimod = head[hv] as usize;
            prev[imod] = pimod as u16;
            head[hv] = imod as u16;

            if wi <= i {
                let rem = s - i;
                if (lc_cnt > 7000 || li > 24576) && (rem > 423 || lst == 0) {
                    pos = wblk(dat, w, 0, syms, &mut lf, &df, eb, li, bs, i - bs, pos);
                    li = 0;
                    lc_cnt = 0;
                    eb = 0;
                    bs = i;
                    let mut j = 0;
                    while j < 286 {
                        lf[j] = 0;
                        j += 1;
                    }
                    j = 0;
                    while j < 30 {
                        df[j] = 0;
                        j += 1;
                    }
                }

                let mut l: usize = 2;
                let mut d: usize = 0;
                let mut ch = c;
                let mut dif = imod.wrapping_sub(pimod) & 32767;

                if rem > 2 && i >= dif {
                    let prev_pos = i - dif;
                    let prev_hv = ((dat[prev_pos] as usize)
                        ^ ((dat[prev_pos + 1] as usize) << bs1)
                        ^ ((dat[prev_pos + 2] as usize) << bs2))
                        & msk;
                    if hv == prev_hv {
                        let maxn = (if n < rem { n } else { rem }) - 1;
                        let maxd = if 32767 < i { 32767 } else { i };
                        let ml = if 258 < rem { 258 } else { rem };

                        while dif <= maxd {
                            ch -= 1;
                            if ch == 0 || imod == pimod {
                                break;
                            }
                            if dat[i + l] == dat[i + l - dif] {
                                let mut nl: usize = 0;
                                while nl < ml && dat[i + nl] == dat[i + nl - dif] {
                                    nl += 1;
                                }
                                if nl > l {
                                    l = nl;
                                    d = dif;
                                    if nl > maxn {
                                        break;
                                    }
                                    let mmd = if dif < nl - 2 { dif } else { nl - 2 };
                                    let mut md: usize = 0;
                                    let mut j: usize = 0;
                                    while j < mmd {
                                        let ti = (i - dif + j) & 32767;
                                        let pti = prev[ti] as usize;
                                        let cd = ti.wrapping_sub(pti) & 32767;
                                        if cd > md {
                                            md = cd;
                                            pimod = ti;
                                        }
                                        j += 1;
                                    }
                                }
                            }
                            imod = pimod;
                            pimod = prev[imod] as usize;
                            dif += imod.wrapping_sub(pimod) & 32767;
                        }
                    }
                }

                if d != 0 {
                    let rfl = REVFL[l] as u32;
                    let rfd = revfd(d);
                    syms[li] = 268435456u32 | (rfl << 18) | rfd;
                    li += 1;
                    let lin = (rfl & 31) as usize;
                    let din = (rfd & 31) as usize;
                    eb += (FLEB[lin] as usize) + (FDEB[din] as usize);
                    lf[257 + lin] += 1;
                    df[din] += 1;
                    wi = i + l;
                    lc_cnt += 1;
                } else {
                    let b = dat[i];
                    syms[li] = b as u32;
                    li += 1;
                    lf[b as usize] += 1;
                }
            }
            i += 1;
        }

        if wi > i {
            i = wi;
        }
        while i < s {
            let b = dat[i];
            syms[li] = b as u32;
            li += 1;
            lf[b as usize] += 1;
            i += 1;
        }

        pos = wblk(dat, w, lst, syms, &mut lf, &df, eb, li, bs, i - bs, pos);
        if lst == 0 {
            st.r = (pos & 7) | ((w[pos >> 3] as usize) << 3);
            pos -= 7;
            st.h = Some(head_vec);
            st.p = Some(prev_vec);
            st.i = i;
            st.w = wi;
        }
    } else {
        let w: &mut [u8] = &mut o[pre..pre + w_len];
        let mut i = st.w;
        let limit = s + (lst as usize);
        while i < limit {
            let mut e = i + 65535;
            if e >= s {
                w[pos >> 3] = lst as u8;
                e = s;
            }
            pos = wfblk(w, pos + 1, &dat[i..e]);
            i += 65535;
        }
        st.i = s;
    }

    o.truncate(pre + shft(pos) + post);
    o
}

/// Deflate with options (fflate `dopt`).
pub fn dopt(
    dat: &[u8],
    opt: &CompressOptions,
    pre: usize,
    post: usize,
    st: Option<&mut DeflateState>,
) -> Vec<u8> {
    let lvl = opt.level.unwrap_or(6);
    match st {
        Some(state) => {
            let plvl = match opt.mem {
                Some(m) => 12 + (m as usize),
                None => {
                    if state.l != 0 {
                        default_plvl(dat.len())
                    } else {
                        20
                    }
                }
            };
            dflt(dat, lvl, plvl, pre, post, state)
        }
        None => {
            let mut state = DeflateState::oneshot();
            if let Some(ref full_dict) = opt.dictionary {
                let dlen = full_dict.len();
                let dict = if dlen > 32768 {
                    &full_dict[dlen - 32768..]
                } else {
                    full_dict.as_slice()
                };
                let mut combined = Vec::with_capacity(dict.len() + dat.len());
                combined.extend_from_slice(dict);
                combined.extend_from_slice(dat);
                state.w = dict.len();
                let plvl = match opt.mem {
                    Some(m) => 12 + (m as usize),
                    None => default_plvl(combined.len()),
                };
                dflt(&combined, lvl, plvl, pre, post, &mut state)
            } else {
                let plvl = match opt.mem {
                    Some(m) => 12 + (m as usize),
                    None => default_plvl(dat.len()),
                };
                dflt(dat, lvl, plvl, pre, post, &mut state)
            }
        }
    }
}

/// One-shot raw DEFLATE compression (fflate `deflateSync`).
pub fn deflate_sync(data: &[u8], opts: &CompressOptions) -> Vec<u8> {
    dopt(data, opts, 0, 0, None)
}
