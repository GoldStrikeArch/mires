//! The host's own data structures, as Rust types.
//!
//! A program compiled by `rustc_codegen_mires` runs on a JavaScript engine, and that engine already
//! ships hash tables: `Map` and the string interning behind every object key. This crate hands
//! those to Rust as [`collections::HashMap`] and [`collections::HashSet`].
//!
//! # Why not `hashbrown`
//!
//! `std`'s `HashMap` is `hashbrown`, and `hashbrown` cannot be compiled by this backend. Its group
//! scan reads sixteen control bytes as one wide integer and punnes the result back to a bitmask,
//! and its one allocation holds control bytes and table entries at two different element
//! granularities. Both are byte-level reinterpretations of a buffer, and in this value model a
//! heap block is a JavaScript array of one element type (`CONTRACT.md`, "Allocation"): a block is
//! retyped once, and reading the same bytes at a second width is a run time refusal rather than a
//! wrong number. So the table has to come from somewhere else, and the host has one.
//!
//! # What a map is at run time
//!
//! A [`collections::HashMap`] **is** a host `Map`. There is no handle table, no side allocation and
//! no `Drop`: the value the backend keeps in the Rust local is the `Map` object itself, and the
//! engine's garbage collector frees it when nothing names it any more. See `CONTRACT.md`,
//! "Host collections".

#![no_std]

extern crate alloc;

pub mod collections;
