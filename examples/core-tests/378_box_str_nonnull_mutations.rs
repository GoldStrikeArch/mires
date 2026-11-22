//! In-place mutations through `Box<str>` and `NonNull<str>`, `NonNull::from(&str)`,
//! `as_bytes_mut` ASCII->UTF-8 mutation, and `PartialEq`/`Ord`/`Hash`/`Display`/`Debug` (MIRES-011).

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use core::hash::{Hash, Hasher};
use core::hint::black_box;
use core::ptr::NonNull;

struct FnvHasher(u64);

impl FnvHasher {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Hasher for FnvHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}

fn hash_of<T: Hash + ?Sized>(v: &T) -> u64 {
    let mut h = FnvHasher::new();
    v.hash(&mut h);
    h.finish()
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // 1. Box<str>::from -> make_ascii_lowercase -> &mut *b make_ascii_uppercase -> drop
    {
        let mut b: Box<str> = Box::from(black_box("AbC-é"));
        print_str(&b);
        b.make_ascii_lowercase();
        print_str(&b);
        {
            let ms: &mut str = &mut *b;
            ms.make_ascii_uppercase();
            print_str(&*ms);
        }
        print_str(&b);
    }

    // 2. String::into_boxed_str -> make_ascii_lowercase -> Box::into_string -> drop
    {
        let mut b: Box<str> = String::from(black_box("AbC-é")).into_boxed_str();
        print_str(&b);
        b.make_ascii_lowercase();
        print_str(&b);
        let s: String = b.into_string();
        print_str(s.as_str());
    }

    // 3. NonNull::from(&mut str) -> NonNull::as_mut() mutation
    {
        let mut s1 = String::from(black_box("HeLLo-é"));
        unsafe {
            let mut nn: NonNull<str> = NonNull::from(s1.as_mut_str());
            nn.as_mut().make_ascii_lowercase();
            print_str(nn.as_ref());
            let nn2 = NonNull::from(s1.as_mut_str());
            #[allow(ambiguous_wide_pointer_comparisons)]
            print_bool(nn == nn2);
            print_bool(core::ptr::eq(nn.as_ptr(), nn2.as_ptr()));
        }
        print_str(s1.as_str());
    }

    // 4. NonNull::new(*mut str) -> (*nn.as_ptr()).make_ascii_uppercase()
    {
        let mut s2 = String::from(black_box("wOrLd"));
        unsafe {
            let nn: NonNull<str> = NonNull::new(s2.as_mut_str() as *mut str).unwrap();
            (*nn.as_ptr()).make_ascii_uppercase();
            print_str(nn.as_ref());
        }
        print_str(s2.as_str());
    }

    // 5. NonNull::from(&str) (immutable &str: empty, ASCII, UTF-8) -> as_ref()
    {
        let lit_ascii: &str = black_box("immutable-lit");
        let lit_utf8: &str = black_box("café-🚀");
        let lit_empty: &str = black_box("");
        let nn_ascii: NonNull<str> = NonNull::from(lit_ascii);
        let nn_utf8: NonNull<str> = NonNull::from(lit_utf8);
        let nn_empty: NonNull<str> = NonNull::from(lit_empty);
        unsafe {
            print_str(nn_ascii.as_ref());
            print_str(nn_utf8.as_ref());
            print_usize(nn_empty.as_ref().len());
            print_bool(nn_empty.as_ref().is_empty());
        }
        // Verify original &str literals and their byte views remain intact
        print_str(lit_ascii);
        print_usize(lit_ascii.as_bytes().len());
        print_str(lit_utf8);
        print_usize(lit_utf8.as_bytes().len());
    }

    // 6. Mutating an ASCII Box<str> into non-ASCII UTF-8 bytes via as_bytes_mut()
    //    (guards against false KnownAscii assumptions)
    {
        let mut b: Box<str> = Box::from(black_box("ab"));
        print_bool(b.is_ascii());
        unsafe {
            let bytes = b.as_bytes_mut();
            bytes[0] = 0xc3;
            bytes[1] = 0xa9; // "é"
        }
        print_bool(b.is_ascii());
        print_usize(b.len());
        print_usize(b.chars().count());
        print_str(&b);
    }

    // 7. split_at_mut / get_mut through Box<str>
    {
        let mut b: Box<str> = Box::from(black_box("AbéCd"));
        let (left, right) = b.split_at_mut(4);
        left.make_ascii_lowercase();
        right.make_ascii_uppercase();
        print_str(&b);
        if let Some(sub) = b.get_mut(0..2) {
            sub.make_ascii_uppercase();
        }
        print_str(&b);
    }

    // 8. PartialEq, Ord, Hash, Display, Debug on Box<str>
    {
        let a: Box<str> = Box::from(black_box("alpha"));
        let b: Box<str> = Box::from(black_box("beta"));
        let a2: Box<str> = Box::from(black_box("alpha"));
        print_bool(a == a2);
        print_bool(a != b);
        print_ordering(a.cmp(&b));
        print_bool(hash_of(&a) == hash_of("alpha"));
        let formatted = format!("{}|{:?}", a, b);
        print_str(formatted.as_str());
    }

    // 9. Safe read-only `is_ascii()` on fresh `Box<str>` and `&mut str` must commit
    //    `block.fresh = false` via `byte_slice_ref` and never corrupt the live buffer
    //    under MIR inlining (`unscale_scalar` u32 scan).
    {
        let b1: Box<str> = Box::from(black_box("abcdé"));
        print_bool(b1.is_ascii());
        print_str(&b1);
        print_bool(&*b1 == "abcdé");

        let b2: Box<str> = Box::from(black_box("abcd"));
        print_bool(b2.is_ascii());
        print_str(&b2);
        print_bool(&*b2 == "abcd");

        let b3: Box<str> = Box::from(black_box("héllo world!"));
        print_bool(b3.is_ascii());
        print_str(&b3);
        print_bool(&*b3 == "héllo world!");

        let mut s = String::from(black_box("pqrsé"));
        print_bool(s.as_mut_str().is_ascii());
        print_str(s.as_str());
    }
}
