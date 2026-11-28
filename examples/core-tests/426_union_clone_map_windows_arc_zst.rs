//! Step 1 regressions:
//! 1. `str::escape_debug` (cloning `NominalForm::Union` with uninitialized/undefined variants)
//! 2. `iter::adapters::map_windows` (comparing nested array slices `[[char; 2]]` & `[[usize; 3]]`)
//! 3. `Arc<NoopWaker>` ZST `Waker` + `core::future::join!` (`ArcInner<ZST>::data` byte-offset addr)

#![no_std]
#![no_main]
#![feature(iter_map_windows, future_join)]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;

use alloc::boxed::Box;
use alloc::string::ToString;
use alloc::sync::Arc;
use alloc::task::Wake;
use alloc::vec;
use alloc::vec::Vec;
use core::future::{Future, join};
use core::task::{Context, Poll};
use prelude::*;

struct UnfusedIter(usize);

impl Iterator for UnfusedIter {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        let curr = self.0;
        self.0 += 1;
        if curr % 7 == 0 { None } else { Some(curr) }
    }
}

struct NoopWaker;

impl Wake for NoopWaker {
    fn wake(self: Arc<Self>) {}
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // 1. str::escape_debug union clone
    let mut escape_ok = true;
    for (inp, exp) in [
        ("abc", "abc"),
        ("\u{7f}\u{ff}", "\\u{7f}\u{ff}"),
        ("\u{100}\u{ffff}", "\u{100}\\u{ffff}"),
        ("\u{10000}\u{10ffff}", "\u{10000}\\u{10ffff}"),
        ("ab\u{200b}", "ab\\u{200b}"),
        ("\u{10d4ea}\r", "\\u{10d4ea}\\r"),
        ("\u{301}a\u{301}bé\u{e000}", "\\u{301}a\u{301}bé\\u{e000}"),
    ] {
        if inp.escape_debug().to_string() != exp {
            escape_ok = false;
        }
    }
    print_bool(escape_ok);

    // 2. map_windows nested array equality
    let chars_w: Vec<[char; 2]> = "abcd".chars().map_windows(|a: &[char; 2]| *a).collect();
    print_bool(chars_w == vec![['a', 'b'], ['b', 'c'], ['c', 'd']]);

    let mut iter = UnfusedIter(1).map_windows(|a: &[usize; 3]| *a);
    let first: Vec<[usize; 3]> = iter.by_ref().collect();
    let second: Vec<[usize; 3]> = iter.by_ref().collect();
    print_bool(first == vec![[1, 2, 3], [2, 3, 4], [3, 4, 5], [4, 5, 6]]);
    print_bool(second == vec![[8, 9, 10], [9, 10, 11], [10, 11, 12], [11, 12, 13]]);

    // 3. Arc<NoopWaker> ZST Waker + future::join!
    let waker = Arc::new(NoopWaker).into();
    let mut cx = Context::from_waker(&waker);
    let mut fut = Box::pin(async { join!(async { 10_i32 }, async { 20_i32 }).await });
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready((a, b)) => print_i32(a + b),
        Poll::Pending => print_i32(-1),
    }
}
