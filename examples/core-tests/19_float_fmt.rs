//! `Display` and `LowerExp` for floats, against the real `core::fmt`.
//!
//! This is the test that the `[AsciiChar] -> str` cast unblocked, and it is worth saying why a
//! float formatter depended on a cast that formats a `char`.
//!
//! `core`'s float `Display` ends in `fmt::float::float_to_decimal_common`, which hands its digits
//! to `Formatter::pad_formatted_parts`. That function slices its buffers, slicing can panic, and
//! `str::slice_error_fail` formats the offending `char` with `Debug` to say where. `char`'s `Debug`
//! escapes through a buffer of `AsciiChar` and calls `as_str` on it — a cast from a slice of
//! one-byte enum values to a `str`. None of that is reachable at run time for a well formed
//! format, but all of it is on the *static* call graph, so the backend has to be able to lower it
//! before a single float can be printed.
//!
//! The values are chosen to pin the parts of the algorithm that differ between implementations:
//! the shortest round-tripping representation (`0.1 + 0.2`), the decision to print without an
//! exponent however long that gets (`f64::MAX`, `f64::MIN_POSITIVE`), the non-finite spellings,
//! and the sign of a negative zero. JavaScript's own `Number.prototype.toString` disagrees with
//! Rust about nearly all of these, which is the point: what runs here is `core`'s formatter,
//! compiled, not the host's.
//!
//! Everything goes through the same `Sink` `11_fmt` uses, so what is checked is the assembled
//! string rather than the pieces.

#![no_std]
#![no_main]

use core::fmt::Write;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

/// A `fmt::Write` sink over a stack buffer.
///
/// The buffer is large because `f64::MIN_POSITIVE` prints as 300-odd digits and `f64::MAX` as
/// rather more; the copy loop is indexed rather than iterated, which is the shape this backend
/// supports on a slice.
struct Sink {
    buf: [u8; 1024],
    len: usize,
}

impl Sink {
    const fn new() -> Sink {
        Sink { buf: [0; 1024], len: 0 }
    }

    /// The bytes written so far, as the `&str` they spell.
    fn as_str(&self) -> &str {
        unsafe { core::str::from_utf8_unchecked(&self.buf[..self.len]) }
    }

    /// Prints what has been written and empties the sink for the next line.
    fn flush(&mut self) {
        print_str(self.as_str());
        self.len = 0;
    }
}

impl Write for Sink {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let bytes = s.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            self.buf[self.len] = bytes[i];
            self.len += 1;
            i += 1;
        }
        Ok(())
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut out = Sink::new();

    // ------------------------------------------------------------------------- plain Display
    let _ = write!(out, "{}", 1.5f64);
    out.flush();
    let _ = write!(out, "{}", 0.0f64);
    out.flush();
    let _ = write!(out, "{}", 1.0f64);
    out.flush();
    let _ = write!(out, "{}", -2.25f64);
    out.flush();
    let _ = write!(out, "{}", 100.0f64);
    out.flush();
    // A value whose shortest round-tripping form is not its exact one, which is where a formatter
    // that simply truncated would disagree.
    let _ = write!(out, "{}", 0.1f64 + 0.2f64);
    out.flush();
    let _ = write!(out, "{}", 1.0f64 / 3.0f64);
    out.flush();

    // ------------------------------------------------------------------------------- precision
    let _ = write!(out, "{:.3}", 1.0f64 / 3.0f64);
    out.flush();
    let _ = write!(out, "{:.0}", 1.0f64 / 3.0f64);
    out.flush();
    let _ = write!(out, "{:.3}", 2.0f64 / 3.0f64);
    out.flush();
    let _ = write!(out, "{:.5}", 1.5f64);
    out.flush();
    let _ = write!(out, "{:.0}", 2.5f64);
    out.flush();
    let _ = write!(out, "{:.0}", 3.5f64);
    out.flush();
    let _ = write!(out, "{:.2}", -1.005f64);
    out.flush();

    // ------------------------------------------------------------- width, fill, sign and align
    let _ = write!(out, "[{:8.2}]", 3.14159f64);
    out.flush();
    let _ = write!(out, "[{:<8.2}]", 3.14159f64);
    out.flush();
    let _ = write!(out, "[{:^8.2}]", 3.14159f64);
    out.flush();
    let _ = write!(out, "[{:08.2}]", -3.14159f64);
    out.flush();
    let _ = write!(out, "[{:*>9.3}]", 2.5f64);
    out.flush();
    let _ = write!(out, "[{:+.2}]", 3.14159f64);
    out.flush();

    // ---------------------------------------------------------------------------- exponent form
    let _ = write!(out, "{:e}", 1234.5f64);
    out.flush();
    let _ = write!(out, "{:e}", 0.00025f64);
    out.flush();
    let _ = write!(out, "{:e}", 1.0f64);
    out.flush();
    let _ = write!(out, "{:e}", 0.0f64);
    out.flush();
    let _ = write!(out, "{:e}", -7.5f64);
    out.flush();
    let _ = write!(out, "{:.2e}", 1234.5f64);
    out.flush();
    let _ = write!(out, "{:E}", 1234.5f64);
    out.flush();

    // ------------------------------------------------------------------------------- the edges
    // `Display` never chooses an exponent, so these are the long ones: `core` prints every digit
    // of the integer part, and every leading zero of the fraction.
    let _ = write!(out, "{}", f64::MAX);
    out.flush();
    let _ = write!(out, "{}", f64::MIN_POSITIVE);
    out.flush();
    let _ = write!(out, "{:e}", f64::MAX);
    out.flush();
    let _ = write!(out, "{:e}", f64::MIN_POSITIVE);
    out.flush();
    let _ = write!(out, "{}", f64::NAN);
    out.flush();
    let _ = write!(out, "{}", f64::INFINITY);
    out.flush();
    let _ = write!(out, "{}", f64::NEG_INFINITY);
    out.flush();
    // A negative zero keeps its sign, which is the one place `-0.0 == 0.0` is not the whole story.
    let _ = write!(out, "{}", -0.0f64);
    out.flush();
    let _ = write!(out, "{}", 0.0f64 - 0.0f64);
    out.flush();
    let _ = write!(out, "{:.2}", f64::NAN);
    out.flush();
    let _ = write!(out, "{:8.2}", f64::INFINITY);
    out.flush();

    // ------------------------------------------------------------------------------------- f32
    // A narrower mantissa has a shorter round-tripping form, so `0.1f32` and `0.1f64` print
    // differently and the width the formatter reads has to come from the type.
    let _ = write!(out, "{}", 1.5f32);
    out.flush();
    let _ = write!(out, "{}", 0.1f32);
    out.flush();
    let _ = write!(out, "{}", 0.1f64);
    out.flush();
    let _ = write!(out, "{:.3}", 1.0f32 / 3.0f32);
    out.flush();
    let _ = write!(out, "{:e}", 1234.5f32);
    out.flush();
    let _ = write!(out, "{}", f32::NAN);
    out.flush();

    // ------------------------------------------------------------------- several in one template
    let _ = write!(out, "{} {} {}", 1.5f64, -0.25f64, 10.0f64);
    out.flush();
    let _ = write!(out, "{0:.1} {1:.1} {0:.3}", 1.0f64 / 3.0f64, 2.0f64 / 3.0f64);
    out.flush();
    let width = 10;
    let precision = 4;
    let _ = write!(out, "[{:>width$.precision$}]", 1.0f64 / 7.0f64);
    out.flush();

    // ------------------------------------------------------------------------------- parsing
    // The other direction, `str::parse::<f64>()`. `dec2flt` reads its digits eight bytes at a time
    // through `ByteSlice::read_u64`, which is a `[u8; 8]` to `u64` transmute — a `BigInt` here, so
    // the int32 operators the shorter widths use would drop half of every word.
    let parsed = ["1.5", "0", "-2.25", "3.14159", "1e3", "2.5e-3", "inf", "-inf", "NaN", "0.1"];
    let mut index = 0;
    while index < parsed.len() {
        match parsed[index].parse::<f64>() {
            Ok(value) => {
                let _ = write!(out, "{}", value);
                out.flush();
            }
            Err(_) => print_str("err"),
        }
        index += 1;
    }
    // The inputs that are not floats have to be rejected rather than guessed at.
    let rejected = ["", "abc", "1.2.3", "--1", "1e", " 1.5"];
    let mut bad = 0;
    let mut index = 0;
    while index < rejected.len() {
        if rejected[index].parse::<f64>().is_err() {
            bad += 1;
        }
        index += 1;
    }
    print_usize(bad);
    // A round trip: `Display` promises the shortest form that reads back as the same value.
    let _ = write!(out, "{}", "0.30000000000000004".parse::<f64>().unwrap());
    out.flush();
    print_bool("0.1".parse::<f64>().unwrap() + "0.2".parse::<f64>().unwrap() == 0.1f64 + 0.2f64);
    print_bool("1.5".parse::<f32>().unwrap() == 1.5f32);
    print_bool("42".parse::<f64>().unwrap() == 42.0f64);

    // ---------------------------------------------------------- the cast this test exists for
    // `char`'s `Debug`, reached directly rather than through a panic path: the escape iterator
    // builds a buffer of `AsciiChar` and calls `as_str` on it.
    let _ = write!(out, "{:?}", 'x');
    out.flush();
    let _ = write!(out, "{:?}", '\n');
    out.flush();
    let _ = write!(out, "{:?}", '\'');
    out.flush();
    let _ = write!(out, "{:?}", '\u{7}');
    out.flush();
}
