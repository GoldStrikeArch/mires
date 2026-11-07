//! `write!` against the real `core::fmt`: the format-string template, the `Display` impls of the
//! integer, `str` and `char` types, and the width/fill/radix machinery in `Formatter`.
//!
//! Everything is written into a `Sink` — a fixed byte buffer implementing `fmt::Write` — and read
//! back out with `str::from_utf8_unchecked`, so what is printed is the whole formatted string
//! rather than the pieces it was assembled from. That is deliberate: it is the assembly that this
//! test is about.

#![no_std]
#![no_main]

use core::fmt::Write;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

/// A `fmt::Write` sink over a stack buffer.
///
/// The copy loop is indexed rather than iterated: `write_str`'s job here is to be the one place
/// bytes cross from a `&str` into a buffer, and an index loop is the shape this backend supports
/// on a slice.
struct Sink {
    buf: [u8; 512],
    len: usize,
}

impl Sink {
    const fn new() -> Sink {
        Sink { buf: [0; 512], len: 0 }
    }

    /// The bytes written so far, as the `&str` they spell.
    ///
    /// `from_utf8_unchecked` and not `from_utf8`: everything written here came out of a `&str`
    /// already, and the checked form walks the bytes a machine word at a time, which is a
    /// reinterpretation of a byte buffer that this backend's pointer model does not have.
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

struct Celsius(i32);

/// A hand written `Display`, which is what a user's own type reaches the formatter through.
impl core::fmt::Display for Celsius {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}C", self.0)
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut sink = Sink::new();

    // The shape of every `write!`: literal pieces around interpolated arguments.
    let _ = write!(sink, "n={} i={} u={}", 42u32, -7i32, 5usize);
    sink.flush();

    // The integer widths, signed and unsigned, at their extremes.
    let _ = write!(sink, "{} {} {} {}", 0u8, 255u8, i32::MIN, u32::MAX);
    sink.flush();
    let _ = write!(sink, "{} {}", i64::MIN, u64::MAX);
    sink.flush();

    // Radices, with and without the `#` prefix.
    let _ = write!(sink, "{:x} {:X} {:b} {:o}", 255u32, 255u32, 5u32, 64u32);
    sink.flush();
    let _ = write!(sink, "{:#x} {:#X} {:#b} {:#o}", 255u32, 255u32, 5u32, 64u32);
    sink.flush();
    // A negative number in a radix is formatted from its two's complement bits.
    let _ = write!(sink, "{:x} {:b}", -1i32, -2i8);
    sink.flush();

    // Width, fill and alignment, over the three `Display` shapes that pad differently:
    // an integer (`pad_integral`), a `str` and a `char` (`pad`).
    let _ = write!(sink, "[{:>8}][{:<8}][{:^8}]", 42, 42, 42);
    sink.flush();
    let _ = write!(sink, "[{:04}][{:04}][{:+}][{:+}]", 7, -7, 7, -7);
    sink.flush();
    let _ = write!(sink, "[{:>6}][{:<6}][{:^6}]", "hi", "hi", "hi");
    sink.flush();
    let _ = write!(sink, "[{:*>6}][{:-<6}][{:.^6}]", "hi", "hi", "hi");
    sink.flush();
    let _ = write!(sink, "[{:>4}][{:<4}][{:^4}]", 'Z', 'Z', 'Z');
    sink.flush();
    // A width narrower than the value never truncates.
    let _ = write!(sink, "[{:2}][{:2}]", 123456, "abcdef");
    sink.flush();
    // `{:.3}` on a `str` truncates rather than pads.
    let _ = write!(sink, "[{:.3}][{:8.3}]", "abcdefgh", "abcdefgh");
    sink.flush();

    // `str` and `char` arguments, including a multi-byte `char`, which is where the sink's byte
    // buffer and the `&str` it is read back as have to agree about UTF-8.
    let _ = write!(sink, "s={} c={} c={}", "text", 'Z', 'é');
    sink.flush();

    // Explicit argument indices: an argument may be skipped, reordered and used twice.
    let _ = write!(sink, "{0} {1} {0} {2}", "a", "b", 3);
    sink.flush();

    // Named arguments, and an inline captured identifier.
    let count = 9;
    let _ = write!(sink, "{count} {value}", value = "v");
    sink.flush();

    // A width taken from another argument rather than from the format string.
    let _ = write!(sink, "[{:width$}][{:>w$}]", 5, 6, width = 4, w = 3);
    sink.flush();

    // `bool` and a user written `Display`, which reaches the formatter as an ordinary argument.
    let _ = write!(sink, "{} {} {}", true, false, Celsius(-40));
    sink.flush();

    // `{{` and `}}` are the escapes, and a literal piece with no arguments at all still writes.
    let _ = write!(sink, "{{{}}} literal", 1);
    sink.flush();

    // A literal piece longer than 128 bytes: the template holds one long `&str` rather than many,
    // and it has to survive the trip through `Arguments` unchanged.
    let _ = write!(
        sink,
        "0123456789abcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmnopqrstuvwxyz\
         0123456789abcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmnopqrstuvwxyz|{}",
        1
    );
    sink.flush();

    // `Arguments::as_str` is `Some` exactly when the template is one literal piece and nothing
    // else — the "evenness" test on the pieces pointer, which is why a real address is needed for
    // one and a formatted template must not accidentally look like one.
    match format_args!("a literal with no arguments").as_str() {
        Some(text) => print_str(text),
        None => print_str("<none>"),
    }
    print_bool(format_args!("x={}", 1).as_str().is_none());
    print_bool(format_args!("").as_str().is_some());

    // `write_str` and `write_char` on the sink directly, and `write_fmt` through `Arguments`.
    let _ = sink.write_str("direct");
    let _ = sink.write_char('!');
    let _ = sink.write_fmt(format_args!(" {}", 2));
    sink.flush();

    // The formatted length is the sink's length: proof that what was printed is what was written.
    let _ = write!(sink, "{}{}{}", 1, 22, 333);
    print_usize(sink.len);
    sink.flush();
}
