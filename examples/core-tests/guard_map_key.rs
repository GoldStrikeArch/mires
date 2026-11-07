//! A gap that is a **type** error rather than a zombie: an aggregate cannot be a map key.
//!
//! A `mires_js::collections::HashMap` is a host `Map`, which compares keys with SameValueZero.
//! That is by value for a number, a BigInt, a boolean and a string, and object IDENTITY for
//! everything else. A struct key would therefore be stored under the object that carried it and
//! never be found again by an equal one built later: the map would answer `None` to a question it
//! holds the answer to, forever, with nothing reported anywhere.
//!
//! So `MapKey` is sealed, and the refusal lands at the call site with the key type named, before
//! any code is generated. `backend/src/map.rs` carries the same rule again as a post-monomorphized
//! check on the marker, which is unreachable through this crate and is there for a marker called by
//! anything else.
//!
//! What this file pins is the message. If aggregate keys ever gain a meaning here -- a derived key
//! function, say -- this test starts failing by *succeeding*, and the expectation is what has to be
//! rewritten then.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use mires_js::collections::HashMap;

#[derive(PartialEq, Eq)]
struct Point {
    x: i32,
    y: i32,
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut places: HashMap<Point, &str> = HashMap::new();
    places.insert(Point { x: 1, y: 2 }, "origin");
    print_usize(places.len());
}
