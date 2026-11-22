//! Extended `fmt_kernel`: float precision `{:.N}`/`{:+.N}` (`f64`/`f32`), `f32` `{}` shortest,
//! integer hex (`{:x}`/`{:X}`/`{:0Wx}`/`{:0WX}`/`{:#0Wx}`/`{:#0WX}`/`{:Wx}`/`{:#WX}` across
//! `u8..u128`/`i8..i128`), and integer decimal `{:0W}`/`{:+0W}`/`{:W}`/`{:+}` — completely
//! eliminating `core::fmt::write` and `flt2dec` (Grisu/Dragon4).

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::format;
use alloc::string::String;
use core::fmt::Write;
use core::hint::black_box;

#[no_mangle]
pub fn rust_entry() {
    // Exact pilot.rs pattern
    let mut out = String::new();
    let tokens = black_box(42usize);
    let distinct = black_box(17usize);
    let mean = black_box(4.1235f64);
    let hash = black_box(0x00ab_cdefu32);
    let _ = write!(
        out,
        "tokens {}\ndistinct {}\nmean {:.3}\nfnv {:08x}",
        tokens, distinct, mean, hash
    );
    let word = black_box("hello");
    let count = black_box(5usize);
    let share = black_box(5.0f64 / 42.0f64);
    let _ = write!(out, "\n{} {} {:08x} {:.3}", word, count, black_box(0x811c_9dc5u32), share);
    print_str(&out);

    // Round-half-to-even & edge floats (f64)
    for &v in &[
        0.0f64,
        -0.0f64,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        0.5,
        1.5,
        2.5,
        3.5,
        -0.5,
        -2.5,
        0.005,
        0.015,
        0.025,
        0.125,
        1.005,
        1234567890123456789012.0,
        1e-10,
        f64::from_bits(1),
    ] {
        let v = black_box(v);
        print_str(&format!("{:.0}|{:.2}|{:.3}|{:.6}|{:.20}|{:+.2}", v, v, v, v, v, v));
    }

    // f32 shortest `{}` and fixed `{:.N}`
    for &v in &[
        0.0f32,
        -0.0f32,
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        0.1f32,
        0.5f32,
        1.5f32,
        2.5f32,
        1.0f32 / 3.0f32,
        f32::MIN_POSITIVE,
        f32::from_bits(1),
    ] {
        let v = black_box(v);
        print_str(&format!("{}|{:.0}|{:.3}|{:.6}", v, v, v, v));
    }

    // Hex across all integer widths & signs
    print_str(&format!(
        "{:x}|{:X}|{:04x}|{:#06x}|{:#6X}",
        black_box(0u8),
        black_box(255u8),
        black_box(10u8),
        black_box(10u8),
        black_box(10u8)
    ));
    print_str(&format!(
        "{:x}|{:X}|{:04x}|{:#06x}|{:#6x}",
        black_box(-1i8),
        black_box(-128i8),
        black_box(-1i8),
        black_box(-1i8),
        black_box(-1i8)
    ));
    print_str(&format!(
        "{:x}|{:X}|{:04x}|{:#08x}",
        black_box(-1i16),
        black_box(i16::MIN),
        black_box(0x2au16),
        black_box(0x2au16)
    ));
    print_str(&format!(
        "{:x}|{:X}|{:08x}|{:#010x}|{:8x}|{:#010X}",
        black_box(-1i32),
        black_box(i32::MIN),
        black_box(0x1a2bu32),
        black_box(0u32),
        black_box(0x1au32),
        black_box(0x1a2bu32)
    ));
    print_str(&format!(
        "{:x}|{:X}|{:016x}|{:#018x}",
        black_box(-1i64),
        black_box(i64::MIN),
        black_box(0x1234u64),
        black_box(0x1234i64)
    ));
    print_str(&format!(
        "{:x}|{:X}|{:#034x}|{:x}|{:X}",
        black_box(u128::MAX),
        black_box(0xdead_beefu128),
        black_box(0x1au128),
        black_box(-1i128),
        black_box(i128::MIN)
    ));

    // Decimal zero-pad / width / sign_plus
    print_str(&format!(
        "{:05}|{:+05}|{:5}|{:+5}|{:+}:{:05}:{:05}",
        black_box(-42i32),
        black_box(42i32),
        black_box(-42i32),
        black_box(42i32),
        black_box(0i32),
        black_box(i32::MIN),
        black_box(i64::MIN)
    ));
}
