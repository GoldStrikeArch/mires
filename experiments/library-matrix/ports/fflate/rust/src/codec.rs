//! UTF-8 and Latin-1 byte-to-UTF-16LE decoding matching fflate 0.8.3's `strFromU8` (WHATWG
//! `TextDecoder` default semantics: strip single leading UTF-8 BOM, maximal-subpart `U+FFFD`
//! replacement for ill-formed byte sequences; or 1-to-1 Latin-1 byte-to-code-unit mapping).

use alloc::vec::Vec;

#[inline]
fn push_u16_le(out: &mut Vec<u8>, unit: u16) {
    out.push((unit & 0xFF) as u8);
    out.push((unit >> 8) as u8);
}

/// Decodes `data` per `strFromU8(data, latin1)` and returns the resulting JS string's exact
/// UTF-16LE bytes (2 bytes per UTF-16 code unit).
pub fn str_from_u8_utf16le(data: &[u8], latin1: bool) -> Vec<u8> {
    let len = data.len();
    if latin1 {
        let mut out = Vec::with_capacity(len * 2);
        let mut i = 0;
        while i < len {
            out.push(data[i]);
            out.push(0);
            i += 1;
        }
        return out;
    }

    let mut out = Vec::with_capacity(len * 2);
    // WHATWG TextDecoder defaults to ignoreBOM: false -> strip one leading EF BB BF.
    let mut i = if len >= 3 && data[0] == 0xEF && data[1] == 0xBB && data[2] == 0xBF {
        3
    } else {
        0
    };

    while i < len {
        let b0 = data[i];
        if b0 < 0x80 {
            push_u16_le(&mut out, b0 as u16);
            i += 1;
        } else if b0 >= 0xC2 && b0 <= 0xDF {
            if i + 1 < len && (data[i + 1] & 0xC0) == 0x80 {
                let cp = (((b0 & 0x1F) as u32) << 6) | ((data[i + 1] & 0x3F) as u32);
                push_u16_le(&mut out, cp as u16);
                i += 2;
            } else {
                push_u16_le(&mut out, 0xFFFD);
                i += 1;
            }
        } else if b0 >= 0xE0 && b0 <= 0xEF {
            let lo = if b0 == 0xE0 { 0xA0 } else { 0x80 };
            let hi = if b0 == 0xED { 0x9F } else { 0xBF };
            if i + 1 < len && data[i + 1] >= lo && data[i + 1] <= hi {
                if i + 2 < len && (data[i + 2] & 0xC0) == 0x80 {
                    let cp = (((b0 & 0x0F) as u32) << 12)
                        | (((data[i + 1] & 0x3F) as u32) << 6)
                        | ((data[i + 2] & 0x3F) as u32);
                    push_u16_le(&mut out, cp as u16);
                    i += 3;
                } else {
                    // Valid 2-byte prefix of a 3-byte sequence: maximal subpart length 2.
                    push_u16_le(&mut out, 0xFFFD);
                    i += 2;
                }
            } else {
                push_u16_le(&mut out, 0xFFFD);
                i += 1;
            }
        } else if b0 >= 0xF0 && b0 <= 0xF4 {
            let lo = if b0 == 0xF0 { 0x90 } else { 0x80 };
            let hi = if b0 == 0xF4 { 0x8F } else { 0xBF };
            if i + 1 < len && data[i + 1] >= lo && data[i + 1] <= hi {
                if i + 2 < len && (data[i + 2] & 0xC0) == 0x80 {
                    if i + 3 < len && (data[i + 3] & 0xC0) == 0x80 {
                        let cp = (((b0 & 0x07) as u32) << 18)
                            | (((data[i + 1] & 0x3F) as u32) << 12)
                            | (((data[i + 2] & 0x3F) as u32) << 6)
                            | ((data[i + 3] & 0x3F) as u32);
                        let adj = cp - 0x10000;
                        push_u16_le(&mut out, (0xD800 | (adj >> 10)) as u16);
                        push_u16_le(&mut out, (0xDC00 | (adj & 0x3FF)) as u16);
                        i += 4;
                    } else {
                        push_u16_le(&mut out, 0xFFFD);
                        i += 3;
                    }
                } else {
                    push_u16_le(&mut out, 0xFFFD);
                    i += 2;
                }
            } else {
                push_u16_le(&mut out, 0xFFFD);
                i += 1;
            }
        } else {
            push_u16_le(&mut out, 0xFFFD);
            i += 1;
        }
    }

    out
}
