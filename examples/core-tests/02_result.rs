//! `core::result`: the combinators, `?` propagation, and a custom error enum.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

#[derive(Clone, Copy, PartialEq)]
enum Error {
    Negative,
    TooBig,
}

fn checked(x: i32) -> Result<i32, Error> {
    if x < 0 {
        Err(Error::Negative)
    } else if x > 100 {
        Err(Error::TooBig)
    } else {
        Ok(x * 2)
    }
}

/// Two `?`s in a row, so the residual conversion runs twice.
fn twice(x: i32) -> Result<i32, Error> {
    let a = checked(x)?;
    let b = checked(a)?;
    Ok(b + 1)
}

fn describe(e: Error) -> &'static str {
    match e {
        Error::Negative => "negative",
        Error::TooBig => "too big",
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let ok: Result<i32, Error> = Ok(3);
    let err: Result<i32, Error> = Err(Error::TooBig);

    print_bool(ok.is_ok());
    print_bool(err.is_err());
    print_i32(ok.ok().unwrap());
    print_i32(err.unwrap_or(-1));
    print_i32(err.unwrap_or_else(|_| -2));

    print_i32(ok.map(|x| x + 1).ok().unwrap());
    print_i32(ok.and_then(checked).ok().unwrap());
    print_bool(err.map_err(|_| Error::Negative).err().unwrap() == Error::Negative);

    // `Result` to `Option` and back.
    print_i32(ok.ok().unwrap());
    print_bool(err.ok().is_none());
    print_i32(ok.iter().count() as i32);

    // `?` propagation, both arms.
    print_i32(twice(5).ok().unwrap());
    print_str(describe(twice(-1).err().unwrap()));
    print_str(describe(twice(60).err().unwrap()));

    // Matching on the error payload.
    match checked(200) {
        Ok(v) => print_i32(v),
        Err(e) => print_str(describe(e)),
    }

    // A `Result` with a unit error, whose layout is a niche.
    let unit: Result<u32, ()> = Ok(9);
    print_u32(unit.ok().unwrap());
    let unit_err: Result<u32, ()> = Err(());
    print_bool(unit_err.is_err());
}
