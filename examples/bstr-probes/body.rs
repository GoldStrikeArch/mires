//! The `js-str` differential probes: pure Rust over `&str`, shared verbatim by the compiled
//! fixture (`probes.rs`) and the native oracle (`native.rs`). Every probe writes one line through
//! `out`; the two runs must print the same lines.
//!
//! The cases are the ones where a `str` held as UTF-16 text and a `str` held as its UTF-8 bytes
//! can disagree: multi-byte scalars (2, 3 and 4 bytes), supplementary characters, ordering of
//! supplementary characters against BMP characters above U+E000 (UTF-16 code-unit order puts a
//! surrogate pair *below* U+E000, UTF-8 byte order puts it above), char boundaries, invalid
//! UTF-8, `chars`/`char_indices`, and formatting with non-ASCII arguments.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// The sample texts: ASCII, a 2-byte, a 3-byte and a 4-byte scalar, and mixes.
pub const SAMPLES: &[&str] = &[
    "plain",
    "é",
    "中",
    "😀",
    "aé中😀b",
    "\u{E000}\u{FFFD}\u{FFFF}",
    "\u{10000}\u{10FFFF}",
    "straße Ünïcödé",
];

/// `count_chars(s)`, by a fold: `Chars::count` reads long text a machine word at a time
/// (`core::str::count`), which this backend refuses at run time in either representation (a
/// byte buffer cannot be re-read at `[u8; 4]` chunk granularity), so the probes count by hand.
pub fn count_chars(s: &str) -> usize {
    s.chars().fold(0, |n, _| n + 1)
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::new();
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn ordering(o: core::cmp::Ordering) -> &'static str {
    match o {
        core::cmp::Ordering::Less => "<",
        core::cmp::Ordering::Equal => "=",
        core::cmp::Ordering::Greater => ">",
    }
}

/// Every in-Rust probe, one line each.
pub fn inside(out: &mut dyn FnMut(&str)) {
    // Lengths, scalar counts, bytes.
    for (n, s) in SAMPLES.iter().enumerate() {
        out(&format!(
            "len[{n}] {} chars {} bytes {}",
            s.len(),
            count_chars(s),
            hex(s.as_bytes())
        ));
    }

    // `chars` and `char_indices`, forwards and backwards.
    let mixed = "aé中😀b";
    let mut line = String::from("char_indices");
    for (i, c) in mixed.char_indices() {
        line.push_str(&format!(" {i}:{c}:{:x}", c as u32));
    }
    out(&line);
    let reversed: String = mixed.chars().rev().collect();
    out(&format!("rev {reversed} {}", reversed.len()));
    let mut line = String::from("char_indices_rev");
    for (i, c) in mixed.char_indices().rev() {
        line.push_str(&format!(" {i}:{}", c.len_utf8()));
    }
    out(&line);

    // Char boundaries and non-panicking slicing.
    let mut line = String::from("boundary");
    for i in 0..=mixed.len() + 1 {
        line.push(if mixed.is_char_boundary(i) { 'T' } else { 'f' });
    }
    out(&line);
    let mut line = String::from("get");
    for (a, b) in [
        (0, 1),
        (0, 2),
        (1, 3),
        (3, 6),
        (6, 9),
        (6, 10),
        (10, 11),
        (1, 2),
        (7, 10),
    ] {
        match mixed.get(a..b) {
            Some(s) => line.push_str(&format!(" [{a}..{b}]={s}")),
            None => line.push_str(&format!(" [{a}..{b}]=None")),
        }
    }
    out(&line);
    out(&format!(
        "slice {}|{}|{}|{}",
        &mixed[1..3],
        &mixed[3..],
        &mixed[..6],
        &mixed[6..10]
    ));

    // Byte access through `as_bytes`.
    let bytes = mixed.as_bytes();
    let mut sum: u32 = 0;
    for i in 0..bytes.len() {
        sum = sum * 31 + u32::from(bytes[i]);
        sum %= 1_000_003;
    }
    out(&format!(
        "bytes {} {} {}",
        bytes[1],
        bytes[bytes.len() - 2],
        sum
    ));

    // Ordering: supplementary characters against BMP characters above U+E000.
    let pairs = [
        ("\u{E000}", "\u{10000}"),
        ("\u{FFFD}", "😀"),
        ("\u{FFFF}", "\u{10000}"),
        ("a\u{FFFF}", "a\u{10FFFF}"),
        ("é", "e"),
        ("中", "😀"),
        ("😀", "😀"),
        ("abc", "abd"),
        ("ab", "abc"),
    ];
    for (a, b) in pairs {
        out(&format!(
            "cmp {a} {b} {} lt {} eq {} max {}",
            ordering(a.cmp(b)),
            a < b,
            a == b,
            core::cmp::max(a, b)
        ));
    }
    let mut words: Vec<&str> = alloc::vec![
        "😀",
        "\u{E000}",
        "z",
        "\u{FFFD}",
        "é",
        "\u{10000}",
        "中",
        "a",
        "\u{FFFF}"
    ];
    words.sort();
    out(&format!("sort {}", words.join(",")));
    words.sort_unstable_by(|a, b| b.cmp(a));
    out(&format!("sort_desc {}", words.join(",")));
    let mut owned: Vec<String> = words.iter().map(|w| w.to_string()).collect();
    owned.sort();
    out(&format!("sort_owned {}", owned.join(",")));

    // Searching with multi-byte patterns.
    let hay = "naïve café, naïve 😀 café";
    out(&format!(
        "search {} {} {:?} {:?} {:?} {:?} {}",
        hay.starts_with("naï"),
        hay.ends_with("café"),
        hay.find("café"),
        hay.rfind("naïve"),
        hay.find('😀'),
        hay.find("zz"),
        hay.contains("é, n")
    ));
    let parts: Vec<&str> = hay.split("é").collect();
    out(&format!("split {} {}", parts.len(), parts.join("|")));
    out(&format!("replace {}", hay.replace("naïve", "NAÏVE")));
    out(&format!(
        "strip {:?} {:?}",
        hay.strip_prefix("naï"),
        hay.strip_suffix("é")
    ));

    // Invalid UTF-8. (`String::from_utf8_lossy` is not probed: its `Utf8Chunks::next` fails at
    // run time with "bad block 1" in either representation, a lowering defect of its own.)
    let cases: [&[u8]; 6] = [
        b"ok",
        &[0x61, 0xc3, 0x28],
        &[0xf0, 0x9f, 0x98],
        &[0xed, 0xa0, 0x80],
        &[0xc0, 0x80],
        &[0xe4, 0xb8, 0xad, 0xff],
    ];
    for (n, case) in cases.iter().enumerate() {
        match core::str::from_utf8(case) {
            Ok(s) => out(&format!("utf8[{n}] ok {s}")),
            Err(e) => out(&format!(
                "utf8[{n}] err {} {:?}",
                e.valid_up_to(),
                e.error_len()
            )),
        }
    }
    match String::from_utf8(alloc::vec![0xe4, 0xb8, 0xad, 0x21]) {
        Ok(s) => out(&format!("from_utf8 ok {s} {}", s.len())),
        Err(_) => out("from_utf8 err"),
    }

    // Formatting with non-ASCII arguments, on the native and the `core::fmt` paths.
    out(&format!("fmt {}|{}|{}|{}", "日本", 'ü', 'ß', "é😀"));
    out(&format!("fmt_debug {:?} {:?}", "é\u{1F600}\t", 'ñ'));
    out(&format!(
        "fmt_width [{:>6}] [{:<4}] [{:^5}]",
        "é中", 'ü', "😀"
    ));
    let n = 42;
    out(&format!("fmt_mixed {n}é{}{n}", '中'));

    // Building strings.
    let mut s = String::from("a");
    s.push('é');
    s.push('😀');
    s.push_str("中b");
    out(&format!("push {s} {}", s.len()));
    let popped = s.pop();
    let popped2 = s.pop();
    out(&format!("pop {popped:?} {popped2:?} {s} {}", s.len()));
    s.insert(1, 'ß');
    s.insert_str(0, "Ω");
    out(&format!("insert {s} {}", s.len()));
    let removed = s.remove(2);
    out(&format!("remove {removed} {s} {}", s.len()));
    s.truncate(4);
    out(&format!("truncate {s} {}", s.len()));
    out(&format!(
        "case {} {}",
        "straße".to_uppercase(),
        "ÀÉÎ".to_lowercase()
    ));
    out(&format!(
        "trim [{}] [{}]",
        "\u{3000}\u{a0}x y\u{2003}".trim(),
        "\u{a0}é".trim_start()
    ));
    let words: Vec<&str> = "a\u{3000}b\u{a0}c d".split_whitespace().collect();
    out(&format!("split_whitespace {}", words.join("|")));
    let mut buffer = [0u8; 4];
    let encoded = '😀'.encode_utf8(&mut buffer);
    out(&format!(
        "encode_utf8 {} {}",
        encoded.len(),
        hex(encoded.as_bytes())
    ));
    out(&format!(
        "char {:?} {:?} {}",
        char::from_u32(0x1f600),
        char::from_u32(0xd800),
        'é'.to_string().repeat(3)
    ));

    // `str::is_ascii` and `<[u8]>::is_ascii` from every byte offset of a word (MIRES-018: `core`'s
    // scan reads its first and last word unaligned).
    let mut line = String::from("is_ascii");
    for s in [
        "The quick brown fox jumps over the lazy dog",
        "a\u{e9} quick brown fox jumps",
        "\u{1f600}abcdefghijk",
        "The quick brown fox jumps \u{f6}",
        "",
    ] {
        line.push(' ');
        for start in 0..6 {
            if let Some(part) = s.get(start..) {
                line.push(if part.is_ascii() { 'T' } else { 'f' });
                line.push(if part.as_bytes().is_ascii() { 'T' } else { 'f' });
                let owned = String::from(part);
                line.push(if owned.is_ascii() { 'T' } else { 'f' });
            }
        }
    }
    out(&line);
}

/// Host-boundary probes over one `&str` the host passed: `(name, answer)` pairs.
pub fn host_answers(s: &str) -> Vec<(&'static str, String)> {
    let tail = &s[s.char_indices().nth(1).map_or(s.len(), |(i, _)| i)..];
    alloc::vec![
        ("len", s.len().to_string()),
        ("chars", count_chars(s).to_string()),
        ("tail", tail.to_string()),
        ("upper", s.to_uppercase()),
        (
            "first",
            s.chars()
                .next()
                .map_or(String::from("-"), |c| format!("{:x}", c as u32))
        ),
    ]
}

/// What `bstr_tail` answers: the text after its first scalar.
pub fn tail(s: &str) -> &str {
    &s[s.char_indices().nth(1).map_or(s.len(), |(i, _)| i)..]
}

/// What `bstr_cmp` answers: `-1`, `0` or `1`.
pub fn compare(a: &str, b: &str) -> i32 {
    a.cmp(b) as i32
}

/// What `bstr_slice` answers: `&s[a..b]`, which panics off a char boundary.
pub fn slice(s: &str, a: usize, b: usize) -> &str {
    &s[a..b]
}
