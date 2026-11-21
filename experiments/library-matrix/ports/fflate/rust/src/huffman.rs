//! Canonical Huffman map construction (`h_map`), tree building & length clamping (`h_tree`),
//! RLE code-length encoding (`lc`), and bit-cost calculation (`clen`), matching fflate 0.8.3
//! bit-for-bit.

use alloc::vec;
use alloc::vec::Vec;

use crate::tables::rev15;

/// Forward (`reverse == false`) or reverse (`reverse == true`) canonical Huffman lookup table
/// (fflate `hMap`).
pub fn h_map(cd: &[u8], mb: usize, reverse: bool) -> Vec<u16> {
    let s = cd.len();
    let mut l = [0u16; 16];
    for &bits in cd {
        if bits != 0 {
            l[(bits - 1) as usize] = l[(bits - 1) as usize].wrapping_add(1);
        }
    }
    let mut le = [0u16; 16];
    let mut i = 1;
    while i < mb {
        le[i] = l[i - 1].wrapping_add(le[i - 1]).wrapping_shl(1);
        i += 1;
    }
    if reverse {
        let mut co = vec![0u16; 1usize << mb];
        let co_sl: &mut [u16] = &mut co;
        let rvb = 15 - mb;
        let mut sym = 0;
        while sym < s {
            let bits = cd[sym] as usize;
            if bits != 0 {
                let sv = ((sym as u16) << 4) | (bits as u16);
                let r1 = mb - bits;
                let mut v = (le[bits - 1] as u32) << r1;
                le[bits - 1] = le[bits - 1].wrapping_add(1);
                let m = v | ((1u32 << r1) - 1);
                while v <= m {
                    let idx = if v < 32768 { (rev15(v) >> rvb) as usize } else { 0 };
                    co_sl[idx] = sv;
                    v += 1;
                }
            }
            sym += 1;
        }
        co
    } else {
        let mut co = vec![0u16; s];
        let co_sl: &mut [u16] = &mut co;
        let mut sym = 0;
        while sym < s {
            let bits = cd[sym] as usize;
            if bits != 0 {
                let code = le[bits - 1] as u32;
                le[bits - 1] = le[bits - 1].wrapping_add(1);
                co_sl[sym] = if code < 32768 {
                    (rev15(code) >> (15 - bits)) as u16
                } else {
                    0
                };
            }
            sym += 1;
        }
        co
    }
}

/// Entry for sorting symbols in `h_tree` (avoids cloning tuples and avoids `core::slice::sort`).
#[derive(Clone, Copy)]
struct SymFreq {
    sym: u16,
    freq: u16,
}

/// Small in-place insertion sort over `<= 288` elements, preserving exact stability. It avoids
/// `core::slice::sort`, whose instantiation is about 15 KB of JavaScript under mires.
fn sort_by_freq_asc(slice: &mut [SymFreq]) {
    let n = slice.len();
    let mut i = 1;
    while i < n {
        let cur = slice[i];
        let mut j = i;
        while j > 0 && slice[j - 1].freq > cur.freq {
            slice[j] = slice[j - 1];
            j -= 1;
        }
        slice[j] = cur;
        i += 1;
    }
}

/// Stable sort by descending `tr[sym]`, then ascending `freq` (ties preserve ascending `sym`).
fn sort_for_clamp(slice: &mut [SymFreq], tr: &[u16]) {
    let n = slice.len();
    let mut i = 1;
    while i < n {
        let cur = slice[i];
        let cur_d = tr[cur.sym as usize];
        let mut j = i;
        while j > 0 {
            let prev = slice[j - 1];
            let prev_d = tr[prev.sym as usize];
            if prev_d < cur_d || (prev_d == cur_d && prev.freq > cur.freq) {
                slice[j] = prev;
                j -= 1;
            } else {
                break;
            }
        }
        slice[j] = cur;
        i += 1;
    }
}

/// Builds canonical Huffman code lengths (max `mb` bits) from a symbol frequency table
/// (fflate `hTree`), returning `(lengths, max_bits)`.
///
/// Uses flat node arrays instead of heap-allocated tree pointers so there is zero per-node
/// allocation overhead.
pub fn h_tree(d: &[u16], mb: usize) -> (Vec<u8>, usize) {
    let mut t = Vec::with_capacity(d.len().min(288));
    let mut i = 0;
    while i < d.len() {
        let f = d[i];
        if f != 0 {
            t.push(SymFreq { sym: i as u16, freq: f });
        }
        i += 1;
    }
    let s = t.len();
    if s == 0 {
        return (Vec::new(), 0);
    }
    if s == 1 {
        let sym = t[0].sym as usize;
        let mut v = vec![0u8; sym + 1];
        v[sym] = 1;
        return (v, 1);
    }

    // t2 preserves ascending symbol order (maxSym is t2[s - 1].sym).
    let mut t2 = t.clone();
    let max_sym = t2[s - 1].sym as usize;

    sort_by_freq_asc(&mut t);

    // Flat pool of nodes: indices 0..s are leaves (in sorted `t` order), index `s` is sentinel
    // with freq 25001, and internal nodes live at `s + 1 .. s + s` (`s - 1` internal nodes).
    // Queue 0 (`i0`) indexes internal nodes via `internal_base + i0`; Queue 2 (`i2`) indexes
    // sorted leaves `0..=s`.
    let sentinel_freq: u32 = 25001;
    let total_nodes = 2 * s;
    let mut node_freq = vec![0u32; total_nodes];
    let mut node_left = vec![0u16; s - 1];
    let mut node_right = vec![0u16; s - 1];

    i = 0;
    while i < s {
        node_freq[i] = t[i].freq as u32;
        i += 1;
    }
    node_freq[s] = sentinel_freq;

    let internal_base = s + 1;
    let mut i0: usize = 0;
    let mut i1: usize = 1;
    let mut i2: usize = 2;

    // First internal node combines leaves 0 and 1.
    node_left[0] = 0;
    node_right[0] = 1;
    node_freq[internal_base] = node_freq[0] + node_freq[1];

    while i1 != s - 1 {
        let l = if node_freq[internal_base + i0] < node_freq[i2] {
            let idx = internal_base + i0;
            i0 += 1;
            idx
        } else {
            let idx = i2;
            i2 += 1;
            idx
        };
        let r = if i0 != i1 && node_freq[internal_base + i0] < node_freq[i2] {
            let idx = internal_base + i0;
            i0 += 1;
            idx
        } else {
            let idx = i2;
            i2 += 1;
            idx
        };
        node_left[i1] = l as u16;
        node_right[i1] = r as u16;
        node_freq[internal_base + i1] = node_freq[l] + node_freq[r];
        i1 += 1;
    }

    // Compute depths top-down from root (`internal_base + i1 - 1`) in reverse creation order
    // (every internal node's children have smaller indices than the node itself!).
    let mut node_depth = vec![0u16; total_nodes];
    let mut k = i1;
    while k > 0 {
        k -= 1;
        let d_child = node_depth[internal_base + k] + 1;
        node_depth[node_left[k] as usize] = d_child;
        node_depth[node_right[k] as usize] = d_child;
    }

    let mut tr = vec![0u16; max_sym + 1];
    let mut mbt: usize = 0;
    i = 0;
    while i < s {
        let dep = node_depth[i];
        tr[t[i].sym as usize] = dep;
        if (dep as usize) > mbt {
            mbt = dep as usize;
        }
        i += 1;
    }

    if mbt > mb {
        let lft = mbt - mb;
        let cst: i32 = 1i32 << lft;
        sort_for_clamp(&mut t2, &tr);
        let mut idx: isize = 0;
        let mut dt: i32 = 0;
        while (idx as usize) < s {
            let sym = t2[idx as usize].sym as usize;
            if (tr[sym] as usize) > mb {
                dt += cst - (1i32 << (mbt - tr[sym] as usize));
                tr[sym] = mb as u16;
                idx += 1;
            } else {
                break;
            }
        }
        dt >>= lft;
        while dt > 0 {
            let sym = t2[idx as usize].sym as usize;
            if (tr[sym] as usize) < mb {
                let old = tr[sym] as usize;
                dt -= 1i32 << (mb - old - 1);
                tr[sym] = (old + 1) as u16;
            } else {
                idx += 1;
            }
        }
        while idx >= 0 && dt != 0 {
            let sym = t2[idx as usize].sym as usize;
            if (tr[sym] as usize) == mb {
                tr[sym] -= 1;
                dt += 1;
            }
            idx -= 1;
        }
        mbt = mb;
    }

    let mut out = vec![0u8; max_sym + 1];
    i = 0;
    while i <= max_sym {
        out[i] = tr[i] as u8;
        i += 1;
    }
    (out, mbt)
}

/// Run-length encodes a code-length array into the RFC 1951 code-length alphabet (fflate `lc`),
/// returning `(encoded_entries, trimmed_symbol_count)`.
pub fn lc(c: &[u8]) -> (Vec<u16>, usize) {
    let mut s = c.len();
    while s > 0 {
        s -= 1;
        if c[s] != 0 {
            break;
        }
    }
    s += 1;

    let mut cl = Vec::with_capacity(s);
    let mut cln: u16 = if c.is_empty() { 0 } else { c[0] as u16 };
    let mut cls: usize = 1;
    let mut i = 1;
    while i <= s {
        let next = if i < c.len() { c[i] as u16 } else { 0 };
        if i != s && next == cln {
            cls += 1;
        } else {
            if cln == 0 && cls > 2 {
                while cls > 138 {
                    cl.push(32754);
                    cls -= 138;
                }
                if cls > 2 {
                    if cls > 10 {
                        cl.push((((cls - 11) << 5) | 28690) as u16);
                    } else {
                        cl.push((((cls - 3) << 5) | 12305) as u16);
                    }
                    cls = 0;
                }
            } else if cls > 3 {
                cl.push(cln);
                cls -= 1;
                while cls > 6 {
                    cl.push(8304);
                    cls -= 6;
                }
                if cls > 2 {
                    cl.push((((cls - 3) << 5) | 8208) as u16);
                    cls = 0;
                }
            }
            while cls > 0 {
                cl.push(cln);
                cls -= 1;
            }
            cls = 1;
            cln = next;
        }
        i += 1;
    }
    (cl, s)
}

/// Total Huffman bit length for frequency table `cf` and code lengths `cl` (fflate `clen`).
#[inline]
pub fn clen(cf: &[u16], cl: &[u8]) -> usize {
    let mut total: usize = 0;
    let mut i = 0;
    while i < cl.len() {
        total += (cf[i] as usize) * (cl[i] as usize);
        i += 1;
    }
    total
}
