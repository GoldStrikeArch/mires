//! Nested array equality lowering through `raw_eq` -> `pod_codec::emit_pod_raw_eq`.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;
use core::num::NonZero;

#[unsafe(no_mangle)]
pub extern "C" fn rust_entry() {
    let a: [[u32; 2]; 3] = black_box([[1, 2], [3, 4], [5, 6]]);
    let b: [[u32; 2]; 3] = black_box([[1, 2], [3, 4], [5, 6]]);
    let c: [[u32; 2]; 3] = black_box([[1, 2], [3, 4], [5, 7]]);
    print_bool(a == b);
    print_bool(a == c);
    print_bool(a != c);

    // 3D nested array
    let x: [[[u8; 2]; 2]; 2] = black_box([[[1, 2], [3, 4]], [[5, 6], [7, 8]]]);
    let y: [[[u8; 2]; 2]; 2] = black_box([[[1, 2], [3, 4]], [[5, 6], [7, 8]]]);
    let z: [[[u8; 2]; 2]; 2] = black_box([[[1, 2], [3, 9]], [[5, 6], [7, 8]]]);
    print_bool(x == y);
    print_bool(x == z);

    // Nested 64-bit integer array
    let l64: [[i64; 2]; 2] = black_box([[-1, 0x1_0000_0001], [42, -99]]);
    let r64: [[i64; 2]; 2] = black_box([[-1, 0x1_0000_0001], [42, -99]]);
    let d64: [[i64; 2]; 2] = black_box([[-1, 0x1_0000_0001], [42, -100]]);
    print_bool(l64 == r64);
    print_bool(l64 == d64);

    // Nested NonZero array
    let n1 = NonZero::new(10u32).unwrap();
    let n2 = NonZero::new(20u32).unwrap();
    let n3 = NonZero::new(30u32).unwrap();
    let na: [[NonZero<u32>; 2]; 2] = black_box([[n1, n2], [n2, n3]]);
    let nb: [[NonZero<u32>; 2]; 2] = black_box([[n1, n2], [n2, n3]]);
    let nc: [[NonZero<u32>; 2]; 2] = black_box([[n1, n2], [n3, n2]]);
    print_bool(na == nb);
    print_bool(na == nc);

    // Nested char and bool arrays
    let ca: [[char; 2]; 2] = black_box([['a', 'λ'], ['🦀', 'z']]);
    let cb: [[char; 2]; 2] = black_box([['a', 'λ'], ['🦀', 'z']]);
    print_bool(ca == cb);

    let ba: [[bool; 3]; 2] = black_box([[true, false, true], [false, false, true]]);
    let bb: [[bool; 3]; 2] = black_box([[true, false, true], [false, false, true]]);
    print_bool(ba == bb);
}
