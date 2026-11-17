//! The native oracle of the `js-str` differential probes (`scripts/bstr-probes.sh`): the same
//! `body.rs`, compiled by stock rustc against `std`, printing the lines `driver.js` prints for
//! the compiled fixture. A host string reaches Rust as `String::from_utf16_lossy` of its UTF-16
//! code units (a lone surrogate is U+FFFD), which is the contract the compiled boundary keeps.

extern crate alloc;

#[path = "body.rs"]
mod body;

/// What `JSON.stringify` prints for a well-formed string: `"`, `\\` and the C0 controls escaped,
/// everything else as it is. (A Rust string has no lone surrogate to escape.)
fn q(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn base(file: &str) -> &str {
    file.rsplit('/').next().unwrap_or(file)
}

/// The host inputs, as UTF-16 code units: `driver.js` builds the same JavaScript strings.
const INPUTS: &[&[u16]] = &[
    &[0x70, 0x6c, 0x61, 0x69, 0x6e],
    &[0xe9],
    &[0xd83d, 0xde00, 0x78],
    &[0x61, 0xd800, 0x62],
    &[0xdc00],
    &[0x78, 0xd83d],
    &[0x4e2d, 0x6587],
    &[0x3a9, 0x2248, 0xe7, 0x221a],
    &[0xfeff, 0x62],
];

const HOST_TEXT: &[u16] = &[0x78, 0xd800, 0x79, 0xd83d, 0xde00];

const CMP: &[(&[u16], &[u16])] = &[
    (&[0xe000], &[0xd800, 0xdc00]),
    (&[0x61], &[0xe9]),
    (&[0xd83d, 0xde00], &[0xfffd]),
    (&[0xffff], &[0xdbff, 0xdfff]),
];

const DISTINCT: &[(&[u16], &[u16])] = &[
    (&[0x78, 0xd800], &[0x78, 0xdbff]),
    (&[0x78, 0xd800], &[0x78, 0xfffd]),
    (&[0xe9], &[0x65, 0x301]),
    (&[0xd83d, 0xde00], &[0xd83d, 0xde00]),
];

const SLICES: &[(&[u16], usize, usize)] = &[
    (&[0x61, 0x62], 0, 2),
    (&[0xe9], 0, 1),
    (&[0x61, 0xd83d, 0xde00], 1, 3),
    (&[0x61, 0xd83d, 0xde00], 1, 5),
];

fn main() {
    std::panic::set_hook(Box::new(|info| {
        if let Some(location) = info.location() {
            println!("panic {}:{}", base(location.file()), location.line());
        }
    }));
    body::inside(&mut |line| println!("{line}"));
    for (n, units) in INPUTS.iter().enumerate() {
        let s = String::from_utf16_lossy(units);
        println!(
            "host[{n}] len {} chars {} echo {} tail {}",
            s.len(),
            body::count_chars(&s),
            q(&s),
            q(body::tail(&s))
        );
        println!("host[{n}] echo_value {}", q(&s));
        for (name, answer) in body::host_answers(&s) {
            println!("host[{n}] {name} {}", q(&answer));
        }
    }
    for (n, (a, b)) in CMP.iter().enumerate() {
        let (a, b) = (String::from_utf16_lossy(a), String::from_utf16_lossy(b));
        println!("host_cmp[{n}] {}", body::compare(&a, &b));
    }
    for (n, (a, b)) in DISTINCT.iter().enumerate() {
        let (a, b) = (String::from_utf16_lossy(a), String::from_utf16_lossy(b));
        let set: std::collections::HashSet<&str> = [a.as_str(), b.as_str()].into_iter().collect();
        println!("distinct[{n}] {}", set.len());
    }
    let text = String::from_utf16_lossy(HOST_TEXT);
    println!("sink {text}|{}", text.len());
    println!("via_host {}", text.len());
    for (n, (units, a, b)) in SLICES.iter().enumerate() {
        let s = String::from_utf16_lossy(units);
        let (a, b) = (*a, *b);
        match std::panic::catch_unwind(|| body::slice(&s, a, b).to_string()) {
            Ok(slice) => println!("slice[{n}] {}", q(&slice)),
            Err(_) => println!("slice[{n}] aborted"),
        }
    }
}
