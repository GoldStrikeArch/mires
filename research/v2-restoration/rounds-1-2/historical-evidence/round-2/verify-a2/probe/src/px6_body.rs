// fmt template reads: 1,000 formatted writes with three arguments (one with a width), then 200 more.
use alloc::string::String;
use core::fmt::Write;
use core::hint::black_box as bb;
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut sink = String::new();
    let mut total = 0usize;
    for i in 0..bb(1000u32) {
        write!(sink, "{} {:>5} {}|", i, i * 3, "x").unwrap();
        if i % 100 == 99 { total += sink.len(); sink.clear(); }
    }
    print_usize(total);
    let mut s = String::new();
    for i in 0..bb(200u32) { write!(s, "{:04}-{}", i, i as u64 * 7).unwrap(); }
    print_usize(s.len());
}
