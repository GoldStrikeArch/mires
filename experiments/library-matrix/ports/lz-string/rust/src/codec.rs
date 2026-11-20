//! Variable-width LZ codes are sent least significant bit first, into words
//! whose first bit occupies the most significant position.
use alloc::{vec, vec::Vec};

#[derive(Clone, Copy, PartialEq)]
pub enum Format {
    Raw,
    Utf16,
    Base64,
    Uri,
    Bytes,
}

const BASE64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/=";
const URI: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+-$";

impl Format {
    fn width(self) -> u32 {
        match self {
            Self::Utf16 => 15,
            Self::Base64 | Self::Uri => 6,
            _ => 16,
        }
    }
}

struct Writer {
    width: u32,
    used: u32,
    word: u16,
    words: Vec<u16>,
}
impl Writer {
    fn bits(&mut self, count: u32, mut value: usize) {
        for _ in 0..count {
            self.word = (self.word << 1) | (value as u16 & 1);
            self.used += 1;
            if self.used == self.width {
                self.words.push(self.word);
                self.word = 0;
                self.used = 0;
            }
            value >>= 1;
        }
    }
}

// Each entry is identified by (prefix code, next UTF-16 unit).
// Prefix zero stores the single-unit entries.
struct Phrases {
    prefix: Vec<usize>,
    unit: Vec<u16>,
    code: Vec<usize>,
}
impl Phrases {
    fn new(n: usize) -> Self {
        let size = (4 * n + 8).next_power_of_two();
        Self {
            prefix: vec![0; size],
            unit: vec![0; size],
            code: vec![0; size],
        }
    }
    fn slot(&self, prefix: usize, unit: u16) -> usize {
        let mask = self.code.len() - 1;
        let mut hash =
            (prefix as u32).wrapping_mul(0x9e3779b1) ^ (unit as u32).wrapping_mul(0x85ebca6b);
        hash ^= hash >> 16;
        let mut i = hash as usize & mask;
        while self.code[i] != 0 && (self.prefix[i] != prefix || self.unit[i] != unit) {
            i = (i + 1) & mask;
        }
        i
    }
    fn insert(&mut self, i: usize, prefix: usize, unit: u16, code: usize) {
        self.prefix[i] = prefix;
        self.unit[i] = unit;
        self.code[i] = code;
    }
}

struct Width {
    bits: u32,
    left: usize,
}
impl Width {
    fn advance(&mut self) {
        self.left -= 1;
        if self.left == 0 {
            self.left = 1 << self.bits;
            self.bits += 1;
        }
    }
}

const USED: u32 = 65536;
fn emit(writer: &mut Writer, width: &mut Width, code: usize, pending: &mut [u32]) {
    let literal = pending[code];
    if literal == USED {
        writer.bits(width.bits, code);
    } else {
        let wide = literal >= 256;
        writer.bits(width.bits, wide as usize);
        writer.bits(if wide { 16 } else { 8 }, literal as usize);
        pending[code] = USED;
        width.advance();
    }
    width.advance();
}

pub fn compress(input: &[u16], format: Format) -> Vec<u16> {
    let mut phrases = Phrases::new(input.len());
    let mut pending = vec![USED; 3];
    // The upstream ordinary JS object silently ignores assignments to __proto__.
    let special = b"__proto__";
    let mut matched: Vec<usize> = vec![0; 3];
    let mut writer = Writer {
        width: format.width(),
        used: 0,
        word: 0,
        words: Vec::new(),
    };
    let mut width = Width { bits: 2, left: 2 };
    let mut prefix = 0;
    for &unit in input {
        let single = phrases.slot(0, unit);
        if phrases.code[single] == 0 {
            phrases.insert(single, 0, unit, pending.len());
            pending.push(unit as u32);
            matched.push(if unit == b'_' as u16 { 1 } else { 0 });
        }
        let literal = phrases.code[single];
        if prefix == 0 {
            prefix = literal;
            continue;
        }
        let i = phrases.slot(prefix, unit);
        if phrases.code[i] != 0 {
            prefix = phrases.code[i];
            continue;
        }
        emit(&mut writer, &mut width, prefix, &mut pending);
        let n = matched[prefix];
        let next = if n > 0 && special[n] as u16 == unit {
            n + 1
        } else {
            0
        };
        if next < special.len() {
            phrases.insert(i, prefix, unit, pending.len());
        }
        matched.push(next);
        pending.push(USED);
        prefix = literal;
    }
    if prefix != 0 {
        emit(&mut writer, &mut width, prefix, &mut pending);
    }
    writer.bits(width.bits, 2);
    writer.bits(writer.width - writer.used, 0);
    let mut output = Vec::new();
    for word in writer.words {
        match format {
            Format::Raw => output.push(word),
            Format::Utf16 => output.push(word + 32),
            Format::Base64 => output.push(BASE64[word as usize] as u16),
            Format::Uri => output.push(URI[word as usize] as u16),
            Format::Bytes => {
                output.push(word >> 8);
                output.push(word & 255);
            }
        }
    }
    if format == Format::Utf16 {
        output.push(32);
    }
    if format == Format::Base64 {
        while output.len() % 4 != 0 {
            output.push(61);
        }
    }
    output
}

struct Reader {
    words: Vec<u16>,
    width: u32,
    position: u32,
    index: usize,
}
impl Reader {
    fn bits(&mut self, count: u32) -> usize {
        let mut value = 0;
        for bit in 0..count {
            let word = self.words.get(self.index).copied().unwrap_or(0);
            self.position -= 1;
            value |= (((word >> self.position) & 1) as usize) << bit;
            if self.position == 0 {
                self.index += 1;
                self.position = self.width;
            }
        }
        value
    }
}

pub fn decompress(input: &[u16], format: Format) -> Option<Vec<u16>> {
    if input.is_empty() {
        return None;
    }
    let mut words = Vec::new();
    if format == Format::Bytes {
        if input.len() % 2 != 0 {
            panic!("odd compressed byte count");
        }
        for pair in input.chunks_exact(2) {
            words.push(pair[0] << 8 | pair[1]);
        }
    } else {
        for &unit in input {
            let word = match format {
                Format::Utf16 => unit.wrapping_sub(32),
                Format::Base64 | Format::Uri => {
                    let ch = if format == Format::Uri && unit == 32 {
                        43
                    } else {
                        unit
                    };
                    let alphabet = if format == Format::Uri { URI } else { BASE64 };
                    alphabet.iter().position(|&b| b as u16 == ch).unwrap_or(0) as u16
                }
                _ => unit,
            };
            words.push(word);
        }
    }
    let mut reader = Reader {
        words,
        width: format.width(),
        position: format.width(),
        index: 0,
    };
    let header = reader.bits(2);
    if header == 2 {
        return Some(Vec::new());
    }
    let mut defined = header != 3;
    let mut previous = Vec::new();
    if defined {
        previous.push(reader.bits(if header == 0 { 8 } else { 16 }) as u16);
    }
    let mut dictionary = vec![Vec::new(); 3];
    dictionary.push(previous.clone());
    let mut output = previous.clone();
    let mut width = Width { bits: 3, left: 4 };
    while reader.index < reader.words.len() {
        let mut code = reader.bits(width.bits);
        if code == 2 {
            return Some(output);
        }
        if code <= 1 {
            let unit = reader.bits(if code == 0 { 8 } else { 16 }) as u16;
            code = dictionary.len();
            dictionary.push(vec![unit]);
            width.advance();
        }
        let entry = if code < dictionary.len() && !dictionary[code].is_empty() {
            dictionary[code].clone()
        } else if code == dictionary.len() {
            if !defined {
                panic!("undefined initial phrase");
            }
            let mut phrase = previous.clone();
            phrase.push(previous[0]);
            phrase
        } else {
            return None;
        };
        output.extend_from_slice(&entry);
        if !defined {
            for &b in b"undefined" {
                previous.push(b as u16);
            }
            defined = true;
        }
        previous.push(entry[0]);
        dictionary.push(previous);
        previous = entry;
        width.advance();
    }
    Some(Vec::new())
}
