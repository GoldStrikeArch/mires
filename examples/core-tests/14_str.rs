//! `str`, which is a JavaScript string on one side and a byte buffer on the other.
//!
//! A `&str` stays a JS string, because that is what makes the emitted code readable and what lets
//! it cross the shim boundary as itself. `core` disagrees: it treats a `str` as UTF-8 bytes and
//! reaches for `as_bytes` almost immediately. The hybrid resolves it — `as_bytes` encodes the
//! string and returns a real `[u8]` slice, `from_utf8_unchecked` decodes one back, and the
//! metadata of a `&str` is its UTF-8 byte length.
//!
//! `as_bytes().len()` is the regression this file exists for. Before the pointer model a `*const
//! u8` from a string was the string itself, `slice_from_raw_parts` built a slice whose `len` field
//! read `undefined`, and the length came out as `undefined` rather than as a number — a **silent**
//! miscompile, the only one the spike ever had. The assertions below pin it.
//!
//! Every length here is a UTF-8 byte count, which is what Rust promises and what JavaScript's own
//! `String.length` (UTF-16 code units) is not: `"héllo"` is 6 bytes, 5 chars and 5 UTF-16 units,
//! while `"日本"` is 6 bytes, 2 chars and 2 units. Both are below.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

/// Words counted over the byte view, which is the substitute for `split(' ')` on a haystack long
/// enough to reach `memchr`'s word-at-a-time path (see the note at `split` below).
fn count_words(text: &str) -> usize {
    let bytes = text.as_bytes();
    let mut count = 0;
    let mut in_word = false;
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b' ' {
            in_word = false;
        } else {
            if !in_word {
                count += 1;
            }
            in_word = true;
        }
        index += 1;
    }
    count
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let ascii = "hello";
    let accented = "héllo";
    let cjk = "日本";
    let empty = "";

    // ------------------------------------------------------------------------------- len
    print_usize(ascii.len());
    print_usize(accented.len());
    print_usize(cjk.len());
    print_usize(empty.len());
    print_bool(ascii.is_empty());
    print_bool(empty.is_empty());
    // A UTF-8 length is not a UTF-16 length: JavaScript would say 5 and 2 for these two.
    print_bool(accented.len() == 6);
    print_bool(cjk.len() == 6);

    // ------------------------------------------------------------------- as_bytes (the regression)
    print_usize(ascii.as_bytes().len());
    print_usize(accented.as_bytes().len());
    print_usize(cjk.as_bytes().len());
    print_usize(empty.as_bytes().len());
    print_bool(ascii.as_bytes().len() == ascii.len());
    print_bool(accented.as_bytes().is_empty() == false);

    print_i32(ascii.as_bytes()[0] as i32);
    print_i32(ascii.as_bytes()[4] as i32);
    print_i32(accented.as_bytes()[1] as i32);
    print_i32(accented.as_bytes()[2] as i32);
    print_i32(cjk.as_bytes()[0] as i32);

    let bytes = ascii.as_bytes();
    print_usize(bytes.len());
    print_i32(bytes[1] as i32);
    print_i32(*bytes.first().unwrap() as i32);
    print_i32(*bytes.last().unwrap() as i32);
    let mut byte_sum = 0i32;
    let mut index = 0;
    while index < bytes.len() {
        byte_sum += bytes[index] as i32;
        index += 1;
    }
    print_i32(byte_sum);

    // ------------------------------------------------------------------------------ bytes()
    print_usize(ascii.bytes().count());
    print_i32(ascii.bytes().map(|b| b as i32).sum::<i32>());
    print_i32(accented.bytes().next().unwrap() as i32);
    print_usize(cjk.bytes().count());
    let mut byte_count = 0;
    for _byte in accented.bytes() {
        byte_count += 1;
    }
    print_i32(byte_count);

    // ------------------------------------------------------------------- chars, char_indices
    print_usize(ascii.chars().count());
    print_usize(accented.chars().count());
    print_usize(cjk.chars().count());
    print_bool(ascii.chars().next().unwrap() == 'h');
    print_bool(cjk.chars().next().unwrap() == '日');
    print_bool(ascii.chars().last().unwrap() == 'o');
    print_bool(ascii.chars().rev().next().unwrap() == 'o');
    print_i32(ascii.chars().map(|c| c as i32).sum::<i32>());

    // `Chars::nth` goes through `advance_by`, which walks a 32 element `[bool; 32]` scratch array
    // built by transmuting an array into an array of `MaybeUninit`. Both the `nth` and the
    // by-hand form are here because that transmute is the interesting part.
    print_bool(accented.chars().nth(1).unwrap() == 'é');
    print_i32(accented.chars().nth(1).unwrap() as i32);
    print_bool(ascii.chars().nth(4).unwrap() == 'o');
    print_bool(ascii.chars().nth(5).is_none());
    let mut accented_chars = accented.chars();
    accented_chars.next();
    print_bool(accented_chars.next().unwrap() == 'é');
    print_i32(accented.chars().skip_while(|c| *c == 'h').next().unwrap() as i32);

    let mut offsets = 0;
    for (offset, character) in accented.char_indices() {
        offsets = offsets * 10 + offset;
        if character == 'l' {
            print_usize(offset);
        }
    }
    print_usize(offsets);
    print_usize(cjk.char_indices().nth(1).unwrap().0);
    print_bool(cjk.char_indices().nth(1).unwrap().1 == '本');
    let mut cjk_indices = cjk.char_indices();
    cjk_indices.next();
    let (second_offset, second_char) = cjk_indices.next().unwrap();
    print_usize(second_offset);
    print_bool(second_char == '本');

    // --------------------------------------------------------------------- equality and ordering
    print_bool(ascii == "hello");
    print_bool(ascii != "world");
    print_bool(ascii == accented);
    print_bool(accented == "héllo");
    print_bool(cjk == "日本");
    print_bool(empty == "");
    print_bool("abc" < "abd");
    print_bool("abc" < "abcd");
    print_bool("Z" < "a");
    print_bool(!("abc" < "abc"));
    print_ordering(ascii.cmp("hello"));
    print_ordering("abc".cmp("abd"));
    print_ordering("abd".cmp("abc"));
    print_ordering("".cmp("a"));

    // A comparison whose two sides are the same bytes but built differently.
    let built = ascii;
    print_bool(built == ascii);

    // ------------------------------------------------------------------------ search predicates
    print_bool(ascii.starts_with("he"));
    print_bool(ascii.starts_with("lo"));
    print_bool(ascii.ends_with("lo"));
    print_bool(accented.starts_with("hé"));
    print_bool(cjk.starts_with('日'));
    print_bool(ascii.starts_with(""));
    print_bool(empty.starts_with(""));
    // `contains` runs the two way `StrSearcher`, which slices with `[a..b]`; slicing can panic,
    // the panic formats the offending `char` with `Debug`, and formatting a `char` escapes it
    // through a buffer of `AsciiChar` whose `as_str` is a cast to `str`. That cast is answered
    // now — a slice of one byte elements decodes to the string its bytes spell, whether those
    // elements are `u8`s or the one byte enum values `AsciiChar` is — so the whole family is in.
    print_bool(ascii.contains("ell"));
    print_bool(ascii.contains("hello"));
    print_bool(ascii.contains("lo"));
    print_bool(ascii.contains("world"));
    print_bool(ascii.contains(""));
    print_bool(ascii.contains("HELLO"));
    print_bool(accented.contains("é"));
    print_bool(accented.contains("hé"));
    print_bool(cjk.contains("本"));
    print_bool(cjk.contains("日本"));
    print_bool(empty.contains("a"));
    print_bool(empty.contains(""));
    // A needle longer than the haystack, and one that matches only at the very end: the two
    // places an off by one in the searcher's window shows up.
    print_bool(ascii.contains("hello world"));
    print_bool(ascii.contains("o"));
    print_bool(ascii.contains('l'));
    print_bool(ascii.contains('z'));

    // `find(char)` uses `CharSearcher`, which walks bytes; `starts_with`/`ends_with` compare a
    // prefix through `as_bytes` and never slice.
    print_bool(ascii.find('l').is_some());
    print_bool(ascii.find('z').is_some());

    match ascii.find('l') {
        Some(offset) => print_usize(offset),
        None => print_str("none"),
    }
    match ascii.find(char::is_numeric) {
        Some(offset) => print_usize(offset),
        None => print_str("none"),
    }
    match ascii.find('z') {
        Some(offset) => print_usize(offset),
        None => print_str("none"),
    }
    match accented.find('l') {
        Some(offset) => print_usize(offset),
        None => print_str("none"),
    }
    // `rfind` is out and does not say so at compile time: `memrchr` reads the haystack a `usize` at
    // a time through `align_to`, and a byte view of a buffer cannot be cast back to one of eight
    // byte elements, so `__rt.unscale` throws at the moment it is reached. `find`'s forward
    // `memchr` takes its short haystack path instead and never asks.

    // ------------------------------------------------------------------------- find(&str)
    // The same `StrSearcher` `contains` runs, asked for the offset rather than for a yes or no.
    match ascii.find("ll") {
        Some(offset) => print_usize(offset),
        None => print_str("none"),
    }
    match ascii.find("hello") {
        Some(offset) => print_usize(offset),
        None => print_str("none"),
    }
    match ascii.find("lo") {
        Some(offset) => print_usize(offset),
        None => print_str("none"),
    }
    match ascii.find("zz") {
        Some(offset) => print_usize(offset),
        None => print_str("none"),
    }
    match ascii.find("") {
        Some(offset) => print_usize(offset),
        None => print_str("none"),
    }
    // A multi-byte needle, whose answer is a byte offset and not a character index.
    match accented.find("llo") {
        Some(offset) => print_usize(offset),
        None => print_str("none"),
    }
    match cjk.find("本") {
        Some(offset) => print_usize(offset),
        None => print_str("none"),
    }
    print_bool(ascii.find("ll") == Some(2));
    print_bool(ascii.find("zz").is_none());

    // -------------------------------------------------------------------------------- slicing
    // `&s[a..b]`, the indexing form, whose failure path is `str::slice_error_fail`: it formats the
    // offending `char` with `Debug`, which escapes it through a buffer of `AsciiChar` and casts
    // that to a `str`. That cast is the one this section used to be unable to reach, so every
    // slice here went through `get` instead. Both forms are exercised now, and `get` stays because
    // the `None` cases below have no panicking spelling.
    print_str(&ascii[1..3]);
    print_str(&ascii[..2]);
    print_str(&ascii[3..]);
    print_str(&ascii[..]);
    print_usize(ascii[1..3].len());
    print_str(&accented[0..3]);
    print_usize(accented[0..3].len());
    print_str(&cjk[3..]);
    print_str(&cjk[..3]);
    print_bool(&ascii[0..0] == "");
    print_bool(&ascii[5..] == "");
    print_bool(&ascii[1..3] == "el");
    // A slice of a slice, so the offsets compose.
    print_str(&(&ascii[1..])[0..2]);
    // Slicing drives the same comparisons and searches the sections above do.
    print_bool((&accented[0..3]).starts_with("h"));
    print_usize((&ascii[1..]).len());

    print_str(ascii.get(1..3).unwrap());
    print_str(ascii.get(..2).unwrap());
    print_str(ascii.get(3..).unwrap());
    print_usize(ascii.get(1..3).unwrap().len());
    print_str(accented.get(0..3).unwrap());
    print_usize(accented.get(0..3).unwrap().len());
    print_str(cjk.get(3..).unwrap());
    print_bool(ascii.get(0..0).unwrap() == "");
    print_bool(accented.get(0..2).is_none());
    print_bool(ascii.get(0..9).is_none());
    print_bool(cjk.get(1..).is_none());

    // ---------------------------------------------------------------------------- split, trim
    print_usize(count_words("the quick brown fox"));
    print_usize(count_words("one"));
    print_usize(count_words(""));
    print_usize(count_words("  spaced  out  "));

    // `split(char)` itself works, and is subject to the same `memchr` boundary `find` is: the
    // searcher reads the haystack a `usize` at a time once it is at least two `usize`s long, and
    // that read goes through `align_to`, which `__rt.unscale` refuses. Below that — every string
    // here — it walks bytes and answers.
    let mut first_word = "";
    for word in "a bc".split(' ') {
        first_word = word;
        break;
    }
    print_str(first_word);
    print_usize("a b c".split(' ').count());
    print_str("a,b".split(',').nth(1).unwrap());

    print_str("  padded  ".trim());
    print_usize("  padded  ".trim().len());
    print_str("left  ".trim_end());
    print_str("  right".trim_start());
    print_str("nothing".trim());
    print_bool("   ".trim().is_empty());
    print_str(" héllo ".trim());

    // ------------------------------------------------------------- back through from_utf8
    let round_tripped = core::str::from_utf8(accented.as_bytes());
    match round_tripped {
        Ok(text) => {
            print_str(text);
            print_usize(text.len());
            print_bool(text == accented);
        }
        Err(_) => print_str("invalid"),
    }
    match core::str::from_utf8(&[0xffu8, 0xfe]) {
        Ok(_) => print_str("valid"),
        Err(_) => print_str("invalid"),
    }
    let raw = [104u8, 105];
    match core::str::from_utf8(&raw) {
        Ok(text) => print_str(text),
        Err(_) => print_str("invalid"),
    }

    // ------------------------------------------------------------------------------- escaping
    // `escape_ascii` builds each output byte by transmuting a `u8` into an `AsciiChar` — a
    // fieldless one byte enum, which is `{ $t: n }` here and the bare byte on a real machine. The
    // escaped forms are what make this worth checking: a tab becomes the two bytes `\` and `t`, so
    // the count below is larger than the input's.
    let mut escaped_sum = 0i32;
    let mut escaped_count = 0usize;
    for byte in b"hi\t!\n".escape_ascii() {
        escaped_sum += byte as i32;
        escaped_count += 1;
    }
    print_i32(escaped_sum);
    print_usize(escaped_count);
    print_usize(b"abc".escape_ascii().count());
    print_usize(b"a\\b".escape_ascii().count());
    print_usize(b"\x00\x1f".escape_ascii().count());
    print_i32(b"\t".escape_ascii().next().unwrap() as i32);

    // `char::escape_debug` is the same machinery reached from the other side, and is what `{:?}`
    // on a `char` runs.
    let mut debug_escaped = 0i32;
    for character in '\n'.escape_debug() {
        debug_escaped = debug_escaped * 7 + character as i32;
    }
    print_i32(debug_escaped);
    print_usize('\n'.escape_debug().count());
    print_usize('x'.escape_debug().count());
    print_usize('\u{7}'.escape_debug().count());
    print_bool('x'.escape_debug().next().unwrap() == 'x');
    print_bool('\n'.escape_debug().next().unwrap() == '\\');

    // A `char`'s own UTF-8 length, which is the same question asked of one code point.
    print_usize('h'.len_utf8());
    print_usize('é'.len_utf8());
    print_usize('日'.len_utf8());
    print_bool('é'.is_alphabetic());
    print_bool('7'.is_ascii_digit());
    print_i32('A' as i32);
}
