//! Safe `&mut str` slicing, reborrowing, formatting, `from_utf8_mut`, and carrier edge cases (MIRES-025).
//!
//! Covers `String::as_mut_str`, `DerefMut`, `split_at_mut`, `split_at_mut_checked`, `get_mut`,
//! `&mut s[a..b]`, reborrowing `&*mut_s` (`len`, `is_empty`, `as_bytes`, `chars`, `is_ascii`,
//! `PartialEq`, `Ord`, `Display`, `Debug`, `format!`), `core::str::from_utf8_mut` (valid &
//! invalid UTF-8 on stack `[u8; N]` and `Vec<u8>`), `<&mut str as Default>::default()`,
//! `Option<&mut str>`, structs/tuples/enums/closures, `mem::swap` on `&mut str`, and
//! loop-carried `&mut str` splitting (exercising slice scalarization).

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::format;
use alloc::string::String;
use alloc::vec;
use core::fmt::Write as _;
use core::hint::black_box;
use core::mem;

struct Holder<'a> {
    tag: u32,
    text: &'a mut str,
}

enum MaybeMut<'a> {
    Empty,
    Live(&'a mut str),
}

#[inline(never)]
fn inspect_and_mutate(s: &mut str) -> String {
    // Reborrow &*s for read-only methods before and after mutation
    let len_before = s.len();
    let empty_before = s.is_empty();
    let ascii_before = s.is_ascii();
    let chars_before = s.chars().count();
    let fmt_before = format!("before=[{}]|{:?}", &*s, s);

    s.make_ascii_uppercase();

    let eq_check = &*s == s.to_ascii_uppercase().as_str();
    let mut out = String::new();
    let _ = write!(
        out,
        "{}:{}:{}:{}:{}|{}|after=[{}]",
        len_before, empty_before, ascii_before, chars_before, eq_check, fmt_before, s
    );
    out
}

#[inline(never)]
fn apply_closure<F: FnMut(&mut str)>(s: &mut str, mut f: F) {
    f(s);
}

#[inline(never)]
fn uppercase_ascii_chunks(mut rest: &mut str) -> usize {
    // Loop-carried &mut str (exercises physical slice lowering)
    let mut count = 0usize;
    while !rest.is_empty() {
        let step = rest.chars().next().unwrap().len_utf8();
        let (head, tail) = rest.split_at_mut(step);
        if count % 2 == 0 {
            head.make_ascii_uppercase();
        } else {
            head.make_ascii_lowercase();
        }
        count += 1;
        rest = tail;
    }
    count
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // 1. Reborrows, Display, Debug, format!, len/is_empty/is_ascii/chars
    let mut s = String::from(black_box("Abc-é-🦀"));
    print_str(inspect_and_mutate(s.as_mut_str()).as_str());
    print_str(s.as_str());

    // 2. split_at_mut & split_at_mut_checked (valid + invalid UTF-8 char boundary)
    let mut s2 = String::from(black_box("AbéCd"));
    // "Ab" (2 bytes) + "é" (2 bytes, indices 2..4) + "Cd" (2 bytes, indices 4..6)
    print_bool(s2.as_mut_str().split_at_mut_checked(3).is_none());
    print_bool(s2.as_mut_str().split_at_mut_checked(99).is_none());
    {
        let (left, right) = s2.split_at_mut(4);
        left.make_ascii_lowercase();
        right.make_ascii_uppercase();
        print_str(&*left);
        print_str(&*right);
    }
    print_str(s2.as_str());

    // 3. get_mut (valid, non-char-boundary, OOB)
    let mut s3 = String::from(black_box("heLLo-é-wOrLd"));
    print_bool(s3.get_mut(6..7).is_none());
    print_bool(s3.get_mut(0..100).is_none());
    if let Some(sub) = s3.get_mut(0..5) {
        sub.make_ascii_uppercase();
    }
    if let Some(sub) = s3.get_mut(8..13) {
        sub.make_ascii_lowercase();
    }
    print_str(s3.as_str());

    // 4. core::str::from_utf8_mut on stack [u8; N] and heap Vec<u8> (valid & invalid)
    let mut arr = *b"StAcK_bUf";
    let arr_s = core::str::from_utf8_mut(black_box(&mut arr[..])).unwrap();
    arr_s.make_ascii_lowercase();
    print_str(&*arr_s);
    print_str(core::str::from_utf8(&arr).unwrap());

    let mut bad = [0xffu8, 0xfeu8, b'A'];
    let err = core::str::from_utf8_mut(black_box(&mut bad[..])).unwrap_err();
    print_usize(err.valid_up_to());

    let mut vec_buf = vec![b'V', b'e', b'C', b'-', 0xc3, 0xa9, b'x'];
    {
        let vs = core::str::from_utf8_mut(vec_buf.as_mut_slice()).unwrap();
        vs.make_ascii_uppercase();
        print_usize(vs.len());
    }
    print_str(core::str::from_utf8(&vec_buf).unwrap());

    // 5. Default::default() for &mut str (empty constant &mut str)
    let empty: &mut str = Default::default();
    empty.make_ascii_lowercase();
    print_usize(empty.len());
    print_bool(empty.is_empty());
    print_usize(empty.as_bytes().len());
    print_str(&*empty);

    // 6. Option<&mut str>, structs, tuples, enums, closures, mem::swap
    let mut a = String::from(black_box("first"));
    let mut b = String::from(black_box("SECOND"));
    {
        let mut r1: &mut str = a.as_mut_str();
        let mut r2: &mut str = b.as_mut_str();
        mem::swap(&mut r1, &mut r2);
        r1.make_ascii_lowercase(); // mutates `b`!
        r2.make_ascii_uppercase(); // mutates `a`!
    }
    print_str(a.as_str());
    print_str(b.as_str());

    let mut h_str = String::from(black_box("HoLdEr"));
    let holder = Holder {
        tag: 7,
        text: h_str.as_mut_str(),
    };
    holder.text.make_ascii_lowercase();
    print_u32(holder.tag);
    print_str(&*holder.text);
    print_str(h_str.as_str());

    let mut e_str = String::from(black_box("EnUm"));
    let mut m: MaybeMut<'_> = black_box(MaybeMut::Live(e_str.as_mut_str()));
    if let MaybeMut::Live(inner) = &mut m {
        inner.make_ascii_uppercase();
    }
    print_str(e_str.as_str());
    let empty_enum: MaybeMut<'_> = black_box(MaybeMut::Empty);
    print_bool(matches!(empty_enum, MaybeMut::Empty));

    let mut opt_str = String::from(black_box("OpTiOn"));
    let mut opt: Option<&mut str> = black_box(Some(opt_str.as_mut_str()));
    if let Some(inner) = opt.as_deref_mut() {
        apply_closure(inner, |t| t.make_ascii_lowercase());
    }
    print_str(opt_str.as_str());

    // 7. Loop-carried &mut str splitting (physical slices)
    let mut alt = String::from(black_box("aBcDeF-é-gHi"));
    let n = uppercase_ascii_chunks(alt.as_mut_str());
    print_usize(n);
    print_str(alt.as_str());
}
