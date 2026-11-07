//! Hash collections backed by the host's `Map`.
//!
//! [`HashMap`] and [`HashSet`] are the two, and both are one host `Map` and nothing else.
//!
//! A doctest cannot run here: these operations only exist once `rustc_codegen_mires` has replaced
//! them, so the block below is prose rather than a test.
//!
//! ```text
//! use mires_js::collections::HashMap;
//!
//! let mut counts: HashMap<&str, u32> = HashMap::new();
//! counts.insert("apples", 3);
//! *counts.get_mut("apples").unwrap() += 1;
//! assert_eq!(counts.get("apples"), Some(&4));
//! ```
//!
//! # Keys
//!
//! A key is anything implementing [`MapKey`], which is sealed: the integers, `bool`, `char`, `str`,
//! `&str` and `String`. Everything else is a compile error at the call site, because a host `Map`
//! compares keys with SameValueZero and that is object *identity* for anything that is not a
//! number, a string, a boolean or a BigInt. A struct key would silently never be found again.
//!
//! Two of those choices are worth stating.
//!
//! * **Floats are not keys.** `f32` and `f64` have no `Eq` in `core` either, and for the same
//!   reason: `NaN != NaN`. (SameValueZero would actually match two NaNs, which is a third answer
//!   again, so excluding them keeps one story.)
//! * **`u64`, `i64`, `u128` and `i128` cross as BigInt** rather than as numbers, because that is
//!   what the value model already spells them as (`CONTRACT.md`, "Value representation").
//!   SameValueZero compares a BigInt by value, so `5u64` finds `5u64`. It does **not** equate `5n`
//!   with `5`, but one map has one key type, so nothing inside a map can disagree.
//!
//! # Keys of one class are interchangeable
//!
//! [`MapKey::Class`] is what a key compares *as*: `String`, `&str` and `str` all compare as `str`,
//! and every other key compares as itself. A lookup takes anything of the map's own class, so a
//! `HashMap<String, V>` is read with `map.get("name")` and never needs a `String` built to ask a
//! question. A `String` key is re-encoded to a host string on every operation that touches it,
//! which is one `__rt.bytes_str` decode: the bytes live in a heap block and the key the host
//! stores is the decoded string, so the two are separate from the moment the key goes in.
//!
//! # Iteration is over a snapshot
//!
//! [`HashMap::iter`] takes the map's keys once, as a host array, and walks that. The map cannot be
//! changed while the iterator is alive (the iterator borrows it), so the snapshot is only visible
//! as a cost: iterating an `n` entry map allocates one host array of `n` keys and does one lookup
//! per key. Order is the host's insertion order, which `Map` guarantees.

mod key;
mod map;
mod raw;
mod set;

pub use key::*;
pub use map::*;
pub use set::*;
