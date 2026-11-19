//! Word frequencies, FNV-1a and a fixed-decimal summary, as specified in
//! libraries/pilot/CONTRACT.md.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

use mires_js::collections::HashMap;

struct Entry {
    word: String,
    count: usize,
    length: usize,
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || !c.is_ascii()
}

/// FNV-1a 32 over the UTF-8 bytes of `text`, wrapping like JavaScript's `>>> 0`.
fn fnv1a(text: &str) -> u32 {
    text.bytes().fold(0x811c_9dc5, |hash, byte| (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193))
}

pub fn analyse(text: &str, top: usize) -> String {
    // Entries in first-occurrence order, and each word's position among them.
    let mut entries: Vec<Entry> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut tokens = 0usize;
    let mut code_points = 0usize;
    let mut word = String::new();
    let mut length = 0usize;

    let mut flush = |word: &mut String, length: &mut usize| {
        if *length == 0 {
            return;
        }
        tokens += 1;
        code_points += *length;
        match index.get(word.as_str()) {
            Some(&at) => entries[at].count += 1,
            None => {
                index.insert(word.clone(), entries.len());
                entries.push(Entry { word: word.clone(), count: 1, length: *length });
            }
        }
        word.clear();
        *length = 0;
    };

    for c in text.chars() {
        if is_word_char(c) {
            // ASCII-only case folding.
            word.push(c.to_ascii_lowercase());
            length += 1;
        } else {
            flush(&mut word, &mut length);
        }
    }
    flush(&mut word, &mut length);

    // `sort_by` is stable: equal keys keep first-occurrence order.
    entries.sort_by(|a, b| b.count.cmp(&a.count).then(b.length.cmp(&a.length)));

    let mean = if tokens == 0 { 0.0 } else { code_points as f64 / tokens as f64 };
    let mut out = String::new();
    let _ = write!(out, "tokens {}\ndistinct {}\nmean {:.3}\nfnv {:08x}", tokens, entries.len(), mean, fnv1a(text));
    for entry in entries.iter().take(top) {
        let share = entry.count as f64 / tokens as f64;
        let _ = write!(out, "\n{} {} {:08x} {:.3}", entry.word, entry.count, fnv1a(&entry.word), share);
    }
    out
}
