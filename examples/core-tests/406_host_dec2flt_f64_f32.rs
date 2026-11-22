//! Host-delegated `dec2flt` (`<f64 as FromStr>::from_str` and `<f32 as FromStr>::from_str`).
//!
//! Verifies exact Rust float grammar, `Empty` vs `Invalid` `ParseFloatError` discrimination,
//! specials (`NaN`, `-nan`, `inf`, `-inf`, `-0.0`), extremes, subnormals, known `f32`
//! double-rounding traps (normal, subnormal, underflow, and overflow midpoints), and indirect
//! function-pointer calls — while dropping `core::num::imp::dec2flt` completely.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;
use core::num::ParseFloatError;
use core::str::FromStr;

fn check_f64(s: &str, empty_err: &ParseFloatError, invalid_err: &ParseFloatError) {
    match black_box(s).parse::<f64>() {
        Ok(v) => {
            if v.is_nan() {
                print_str(if v.is_sign_negative() { "-NaN" } else { "NaN" });
            } else {
                print_u64(v.to_bits());
            }
        }
        Err(ref e) => {
            if e == empty_err {
                print_str("Empty");
            } else if e == invalid_err {
                print_str("Invalid");
            } else {
                print_str("UnknownErr");
            }
        }
    }
}

fn check_f32(s: &str, empty_err: &ParseFloatError, invalid_err: &ParseFloatError) {
    match black_box(s).parse::<f32>() {
        Ok(v) => {
            if v.is_nan() {
                print_str(if v.is_sign_negative() { "-NaN" } else { "NaN" });
            } else {
                print_u32(v.to_bits());
            }
        }
        Err(ref e) => {
            if e == empty_err {
                print_str("Empty");
            } else if e == invalid_err {
                print_str("Invalid");
            } else {
                print_str("UnknownErr");
            }
        }
    }
}

#[no_mangle]
pub fn rust_entry() {
    let empty_err = black_box("").parse::<f64>().unwrap_err();
    let invalid_err = black_box("invalid").parse::<f64>().unwrap_err();
    print_bool(empty_err != invalid_err);

    // Function-pointer invocation exercises the replaced body directly.
    let fp64: fn(&str) -> Result<f64, ParseFloatError> = black_box(f64::from_str);
    let fp32: fn(&str) -> Result<f32, ParseFloatError> = black_box(f32::from_str);
    print_bool(fp64("3.5").unwrap() == 3.5f64);
    print_bool(fp32("3.5").unwrap() == 3.5f32);
    print_bool(fp64("").unwrap_err() == empty_err);
    print_bool(fp32(" 1.0").unwrap_err() == invalid_err);

    let cases: &[&str] = &[
        "",
        " ",
        "+",
        "-",
        ".",
        "1e",
        "1e+",
        "0x10",
        "1_0",
        " 1.0",
        "1.0 ",
        "inf ",
        " nan",
        "infin",
        "0",
        "-0.0",
        "+0.0",
        ".5",
        "5.",
        "007",
        "3.141592653589793",
        "2.5E10",
        "2.5e-10",
        "inf",
        "+inf",
        "-inf",
        "Infinity",
        "-INFINITY",
        "nan",
        "+nan",
        "-nan",
        "NaN",
        // f64 extremes and subnormals
        "1.7976931348623157e308",
        "5e-324",
        "2.4703282292062327e-324",
        "2.4703282292062328e-324",
        "1e400",
        "1e-400",
        "-1e-400",
        // f32 double-rounding traps (normal, overflow, subnormal, underflow)
        "7.038531e-26",
        "1.000000059604644775390625",
        "1.00000005960464477539062500000000000000000000001",
        "1.00000005960464477539062499999999999999999999999",
        "1.000000178813934326171875",
        "1.00000017881393432617187500000000000000000000001",
        "1.00000017881393432617187499999999999999999999999",
        "3.40282356779733661637539395458142568448e38",
        "3.40282356779733661637539395458142568447999999e38",
        "3.40282356779733661637539395458142568448000001e38",
        "7.00649232162408535461864791644958065640130970938257885878534141944895541342930300743319094181060791015625e-46",
        "7.006492321624085354618647916449580656401309709382578858785341419448955413429303007433190941810607910156249e-46",
        "7.006492321624085354618647916449580656401309709382578858785341419448955413429303007433190941810607910156251e-46",
        "-7.006492321624085354618647916449580656401309709382578858785341419448955413429303007433190941810607910156249e-46",
        "2.101947696487225606385594374934874196920392912814773657635602425834686624028790902229957282543182373046875e-45",
        "2.1019476964872256063855943749348741969203929128147736576356024258346866240287909022299572825431823730468749e-45",
        "2.1019476964872256063855943749348741969203929128147736576356024258346866240287909022299572825431823730468751e-45",
    ];

    for &s in cases {
        check_f64(s, &empty_err, &invalid_err);
        check_f32(s, &empty_err, &invalid_err);
    }
}
