//! `str::Lines`, `split_once`, and `char` suffix matching keep and return string slice views through
//! the custom sysroot's safe scalar byte access, without retaining impossible `char` conversion or
//! slice-index failures and the native-memory alignment graph. The companion absent list pins those
//! code-shape gates.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

#[inline(never)]
fn strip_suffix(input: &'static str, suffix: char) -> Option<&'static str> {
    input.strip_suffix(suffix)
}

#[inline(never)]
fn find(input: &'static str, needle: char) -> usize {
    input.find(needle).unwrap_or(usize::MAX)
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let input = "alpha=1\nbeta=two\r\ngamma\n\nlast=four\n";
    let mut lines = 0;
    let mut row_bytes = 0;
    let mut key_bytes = 0;
    let mut value_bytes = 0;
    let mut pairs = 0;
    let mut has_empty = false;
    let mut stripped_carriage_return = true;

    for row in input.lines() {
        lines += 1;
        row_bytes += row.len();
        has_empty |= row.is_empty();
        stripped_carriage_return &= !row.as_bytes().contains(&b'\r');
        let (key, value) = row.split_once('=').unwrap_or((row, ""));
        key_bytes += key.len();
        value_bytes += value.len();
        pairs += usize::from(!value.is_empty());
    }

    print_usize(lines);
    print_usize(row_bytes);
    print_usize(key_bytes);
    print_usize(value_bytes);
    print_usize(pairs);
    print_bool(has_empty);
    print_bool(stripped_carriage_return);

    let widths = "a\u{e9}\u{65e5}\u{672c}\u{1f600}z";
    print_usize(find(widths, 'a'));
    print_usize(find(widths, '\u{e9}'));
    print_usize(find(widths, '\u{672c}'));
    print_usize(find(widths, '\u{1f600}'));
    print_usize(find("\u{0169}\u{e9}", '\u{e9}'));
    print_usize(find("\u{20ac}\u{672c}", '\u{672c}'));
    print_usize(find("\u{1f640}\u{1f600}", '\u{1f600}'));
    print_usize(find("\u{a041}", '\u{a041}'));
    print_bool(find("\u{0169}", '\u{e9}') == usize::MAX);
    print_bool(find("\u{20ac}", '\u{672c}') == usize::MAX);
    print_bool(find("\u{1f640}", '\u{1f600}') == usize::MAX);

    print_usize(strip_suffix("za", 'a').unwrap().len());
    print_usize(strip_suffix("z\u{e9}", '\u{e9}').unwrap().len());
    print_usize(strip_suffix("z\u{672c}", '\u{672c}').unwrap().len());
    print_usize(strip_suffix("z\u{1f600}", '\u{1f600}').unwrap().len());
    print_bool(strip_suffix("z", '\u{e9}').is_none());
    print_bool(strip_suffix("za", 'z').is_none());
}
