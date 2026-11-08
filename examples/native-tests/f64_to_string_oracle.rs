use std::io::{self, BufWriter, Write};

const EXPONENTS: u64 = 1 << 11;
const MANTISSA_BUCKETS: u64 = 1 << 9;
const LOW_MANTISSA_MASK: u64 = (1 << 43) - 1;

fn mix(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn write_case(out: &mut impl Write, bits: u64) -> io::Result<()> {
    writeln!(out, "{bits:016x}\t{}", f64::from_bits(bits))
}

fn main() -> io::Result<()> {
    let directed = [
        0,
        1 << 63,
        1,
        (1 << 63) | 1,
        2,
        (1 << 52) - 1,
        1 << 52,
        (1 << 52) + 1,
        0x7fef_ffff_ffff_ffff,
        0xffef_ffff_ffff_ffff,
        0x7ff0_0000_0000_0000,
        0xfff0_0000_0000_0000,
        0x7ff0_0000_0000_0001,
        0x7ff8_0000_0000_0000,
        0x7fff_ffff_ffff_ffff,
        0xfff0_0000_0000_0001,
        0xfff8_0000_0000_0000,
        0xffff_ffff_ffff_ffff,
        (1e-8_f64).to_bits(),
        (1e-7_f64).to_bits(),
        (1e-6_f64).to_bits(),
        (1e-5_f64).to_bits(),
        (1e19_f64).to_bits(),
        (1e20_f64).to_bits(),
        (1e21_f64).to_bits(),
        (1e22_f64).to_bits(),
        (-1e-8_f64).to_bits(),
        (-1e-7_f64).to_bits(),
        (-1e-6_f64).to_bits(),
        (-1e-5_f64).to_bits(),
        (-1e19_f64).to_bits(),
        (-1e20_f64).to_bits(),
        (-1e21_f64).to_bits(),
        (-1e22_f64).to_bits(),
        (0.1_f64).to_bits(),
        (1.0 / 3.0_f64).to_bits(),
        f64::MIN_POSITIVE.to_bits(),
        f64::MAX.to_bits(),
        f64::EPSILON.to_bits(),
    ];

    let mut out = BufWriter::new(io::stdout().lock());
    for bits in directed {
        write_case(&mut out, bits)?;
    }

    for exponent in 0..EXPONENTS {
        for bucket in 0..MANTISSA_BUCKETS {
            let random = mix((exponent << 32) | bucket);
            let sign = ((exponent ^ bucket) & 1) << 63;
            let mantissa = (bucket << 43) | (random & LOW_MANTISSA_MASK);
            write_case(&mut out, sign | (exponent << 52) | mantissa)?;
        }
    }

    out.flush()
}
