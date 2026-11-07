//! One entry point per shape the `.d.ts` has to describe.
//!
//! The point of the file is the goldens beside it, not the code: every entry exists so that one
//! Rust type appears in `01_shapes.d.ts` and can be read there. `scripts/dts-check.mjs` then
//! asserts the properties that would still hold if the goldens were rewritten carelessly.
//!
//! # What the representation buys, stated as a test rather than as a claim
//!
//! A fieldless enum is its variant's NAME at run time, so it is a union of string literals and
//! TypeScript narrows a `switch` on it. An enum with a payload is an object under `TAG`, so it is
//! a discriminated union and TypeScript narrows that too. Under the old integer representation
//! every one of these would have been `number`, and a `.d.ts` would have said nothing.

#![no_std]
#![no_main]

use js_extern_macro::js_extern;
use view_abi::{JsValue, Node, Sig};

// A declared interface, so the golden carries the module surface as well as the entry points: a
// consumer needs it to build an import map, and it is what `#[js_extern]` adds to a module.
#[js_extern(module = "chart.js")]
unsafe extern "C" {
    #[js(new = "Chart")]
    fn chart_new(target: JsValue) -> JsValue;
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

/// A fieldless enum with no `repr`: the variant's name is the value.
pub enum Direction {
    North,
    South,
}

/// A fieldless enum with an integer `repr`: the value IS the discriminant, which is what makes a
/// transmute to that integer a move with no shape change.
#[repr(u8)]
pub enum Level {
    Low = 1,
    High = 7,
}

/// A payload-carrying enum: an object under `TAG`, including for its empty variant.
pub enum Reply {
    Empty,
    Text(u32),
    Point { x: f64, y: f64 },
}

/// An ordinary struct: an object keyed by field name.
#[repr(C)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

/// The primitives, and the two that are NOT numbers: a `u64` is wider than a double and a `char`
/// is a code point.
#[unsafe(no_mangle)]
pub extern "C" fn primitives(count: u32, big: u64, ratio: f64, flag: bool, letter: char) -> i64 {
    let _ = (count, big, ratio, flag, letter);
    0
}

/// A string crosses as a string; nothing is copied.
#[unsafe(no_mangle)]
pub extern "C" fn greet(name: &str) -> &'static str {
    let _ = name;
    "hello"
}

/// A struct in and a struct out.
#[unsafe(no_mangle)]
pub extern "C" fn shift(point: Point, by: f64) -> Point {
    Point { x: point.x + by, y: point.y + by }
}

/// A fieldless enum both ways.
#[unsafe(no_mangle)]
pub extern "C" fn turn(direction: Direction) -> Direction {
    match direction {
        Direction::North => Direction::South,
        Direction::South => Direction::North,
    }
}

/// A `repr(int)` fieldless enum, which is a number and has no narrower type.
#[unsafe(no_mangle)]
pub extern "C" fn raise(level: Level) -> Level {
    match level {
        Level::Low => Level::High,
        Level::High => Level::High,
    }
}

/// A payload-carrying enum: the discriminated union.
#[unsafe(no_mangle)]
pub extern "C" fn describe(reply: Reply) -> Reply {
    reply
}

/// `Option` is an ordinary payload-carrying enum, and its encoding is TOTAL: `None` is a value and
/// never an absent one.
#[unsafe(no_mangle)]
pub extern "C" fn maybe(value: f64, present: bool) -> Option<f64> {
    match present {
        true => Some(value),
        false => None,
    }
}

/// The runtime handles, which are opaque and must not be told apart from numbers by accident.
#[unsafe(no_mangle)]
pub extern "C" fn bump(count: Sig<f64>) -> Sig<f64> {
    count.set(count.get() + 1.0);
    count
}

/// A `Node`, so the file carries two distinct opaque handles rather than one.
#[unsafe(no_mangle)]
pub extern "C" fn wrap(value: f64) -> Node {
    view_abi::content(value)
}

/// The declared interface, so its import survives into the emitted module.
#[unsafe(no_mangle)]
pub extern "C" fn chart(target: JsValue) -> JsValue {
    chart_new(target)
}

/// A tuple, which is a JavaScript array.
#[unsafe(no_mangle)]
pub extern "C" fn split(value: f64) -> (f64, f64) {
    (value, value)
}
