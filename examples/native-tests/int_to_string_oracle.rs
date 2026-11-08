//! Prints the exact Rust `Display` text of every primitive `to_string` case the differential runs.
//!
//! One line per case: the lane name, the decimal input, and the expected text as UTF-8 hex. Hex
//! keeps a `char` case whose text is a newline or a tab on one line.

use std::io::{self, BufWriter, Write};

/// Random samples per integer width beyond the directed edges.
const SAMPLES: u32 = 20_000;

fn mix(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 = mix(self.0);
        self.0
    }

    fn next_u128(&mut self) -> u128 {
        (u128::from(self.next()) << 64) | u128::from(self.next())
    }
}

fn write_case(out: &mut impl Write, kind: &str, input: &str, text: &str) -> io::Result<()> {
    write!(out, "{kind}\t{input}\t")?;
    for byte in text.as_bytes() {
        write!(out, "{byte:02x}")?;
    }
    writeln!(out)
}

macro_rules! exhaustive {
    ($out:expr, $kind:literal, $ty:ty) => {{
        for value in <$ty>::MIN..=<$ty>::MAX {
            write_case($out, $kind, &value.to_string(), &value.to_string())?;
        }
    }};
}

macro_rules! sweep {
    ($out:expr, $random:expr, $kind:literal, $ty:ty) => {{
        let mut cases: Vec<$ty> = vec![
            <$ty>::MIN,
            <$ty>::MIN + 1,
            <$ty>::MIN / 2,
            0,
            1,
            <$ty>::MAX / 2,
            <$ty>::MAX - 1,
            <$ty>::MAX,
        ];
        let mut power: $ty = 1;
        while let Some(next) = power.checked_mul(10) {
            cases.push(next);
            cases.push(next - 1);
            if let Some(above) = next.checked_add(1) {
                cases.push(above);
            }
            if let Some(negated) = <$ty>::MIN.checked_add(next) {
                if negated != 0 {
                    cases.push(0 - next);
                }
            }
            power = next;
        }
        for _ in 0..SAMPLES {
            cases.push($random.next_u128() as $ty);
        }
        for value in cases {
            write_case($out, $kind, &value.to_string(), &value.to_string())?;
        }
    }};
}

fn main() -> io::Result<()> {
    let mut out = BufWriter::new(io::stdout().lock());
    let mut random = Random(0x5eed_1234_9876_abcd);

    exhaustive!(&mut out, "i8", i8);
    exhaustive!(&mut out, "u8", u8);
    exhaustive!(&mut out, "i16", i16);
    exhaustive!(&mut out, "u16", u16);

    sweep!(&mut out, random, "i32", i32);
    sweep!(&mut out, random, "u32", u32);
    sweep!(&mut out, random, "i64", i64);
    sweep!(&mut out, random, "u64", u64);
    sweep!(&mut out, random, "i128", i128);
    sweep!(&mut out, random, "u128", u128);
    // `isize` and `usize` are 32 bits wide on the compiled target, so their cases are exactly the
    // 32-bit ones. Sampling them through the host's own pointer width would test the wrong range.
    sweep!(&mut out, random, "isize", i32);
    sweep!(&mut out, random, "usize", u32);

    for value in [false, true] {
        write_case(&mut out, "bool", &value.to_string(), &value.to_string())?;
    }

    for point in 0..=0x10_ffff_u32 {
        let Some(value) = char::from_u32(point) else {
            continue;
        };
        write_case(&mut out, "char", &point.to_string(), &value.to_string())?;
    }

    out.flush()
}
