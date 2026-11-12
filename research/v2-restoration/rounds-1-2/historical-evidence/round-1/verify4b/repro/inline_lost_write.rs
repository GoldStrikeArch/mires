//! Verifier repro for Stage 4'b: right with the base linker (c911157), wrong with the change.
//!
//! `set` writes one field through a `&mut` to a `Copy` struct local that was assigned on two
//! paths. Inlined, the callee's parameter becomes a second name for the caller's object
//! (`s = a`), and the finished-JavaScript SROA splits `a` and `s` into separate per-field locals,
//! so the store lands in `s$f` and `a$f` keeps the old value. Native Rust prints 254 on the last
//! line; the change prints 7.

#![no_std]
#![no_main]
#![allow(warnings)]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;

#[derive(Clone, Copy)]
struct S {
    f: u8,
}

fn set(s: &mut S, v: u8) {
    s.f = v;
}

// Four callers with five one-caller helpers each. The base linker charges every inlined site at
// least 24 bytes against a 384-byte ledger (16 sites), so it has spent the ledger on these before
// it reaches `part` and leaves `set` a call there. The arithmetic padding keeps each caller under
// its own twice-the-entry-size limit.
fn a0_h0(x: u32) -> u32 {
    x.wrapping_mul(3) ^ 1
}
fn a0_h1(x: u32) -> u32 {
    x.wrapping_mul(5) ^ 2
}
fn a0_h2(x: u32) -> u32 {
    x.wrapping_mul(7) ^ 3
}
fn a0_h3(x: u32) -> u32 {
    x.wrapping_mul(9) ^ 4
}
fn a0_h4(x: u32) -> u32 {
    x.wrapping_mul(11) ^ 5
}

fn a0_burn() {
    let mut x = black_box(1u32);
    x = x.wrapping_mul(3).wrapping_add(11) ^ (x >> 1);
    x = x.wrapping_mul(5).wrapping_add(12) ^ (x >> 2);
    x = x.wrapping_mul(7).wrapping_add(13) ^ (x >> 3);
    x = x.wrapping_mul(9).wrapping_add(14) ^ (x >> 4);
    x = x.wrapping_mul(11).wrapping_add(15) ^ (x >> 5);
    x = x.wrapping_mul(13).wrapping_add(16) ^ (x >> 6);
    x = x.wrapping_mul(15).wrapping_add(17) ^ (x >> 7);
    x = x.wrapping_mul(17).wrapping_add(18) ^ (x >> 1);
    x = x.wrapping_mul(19).wrapping_add(19) ^ (x >> 2);
    x = x.wrapping_mul(21).wrapping_add(20) ^ (x >> 3);
    x = x.wrapping_mul(23).wrapping_add(21) ^ (x >> 4);
    x = x.wrapping_mul(25).wrapping_add(22) ^ (x >> 5);
    x = x.wrapping_mul(27).wrapping_add(23) ^ (x >> 6);
    x = x.wrapping_mul(29).wrapping_add(24) ^ (x >> 7);
    x = x.wrapping_mul(31).wrapping_add(25) ^ (x >> 1);
    x = x.wrapping_mul(33).wrapping_add(26) ^ (x >> 2);
    x = a0_h0(x);
    x = a0_h1(x);
    x = a0_h2(x);
    x = a0_h3(x);
    x = a0_h4(x);
    print_u32(x);
}

fn a1_h0(x: u32) -> u32 {
    x.wrapping_mul(13) ^ 6
}
fn a1_h1(x: u32) -> u32 {
    x.wrapping_mul(15) ^ 7
}
fn a1_h2(x: u32) -> u32 {
    x.wrapping_mul(17) ^ 8
}
fn a1_h3(x: u32) -> u32 {
    x.wrapping_mul(19) ^ 9
}
fn a1_h4(x: u32) -> u32 {
    x.wrapping_mul(21) ^ 10
}

fn a1_burn() {
    let mut x = black_box(2u32);
    x = x.wrapping_mul(3).wrapping_add(11) ^ (x >> 1);
    x = x.wrapping_mul(5).wrapping_add(12) ^ (x >> 2);
    x = x.wrapping_mul(7).wrapping_add(13) ^ (x >> 3);
    x = x.wrapping_mul(9).wrapping_add(14) ^ (x >> 4);
    x = x.wrapping_mul(11).wrapping_add(15) ^ (x >> 5);
    x = x.wrapping_mul(13).wrapping_add(16) ^ (x >> 6);
    x = x.wrapping_mul(15).wrapping_add(17) ^ (x >> 7);
    x = x.wrapping_mul(17).wrapping_add(18) ^ (x >> 1);
    x = x.wrapping_mul(19).wrapping_add(19) ^ (x >> 2);
    x = x.wrapping_mul(21).wrapping_add(20) ^ (x >> 3);
    x = x.wrapping_mul(23).wrapping_add(21) ^ (x >> 4);
    x = x.wrapping_mul(25).wrapping_add(22) ^ (x >> 5);
    x = x.wrapping_mul(27).wrapping_add(23) ^ (x >> 6);
    x = x.wrapping_mul(29).wrapping_add(24) ^ (x >> 7);
    x = x.wrapping_mul(31).wrapping_add(25) ^ (x >> 1);
    x = x.wrapping_mul(33).wrapping_add(26) ^ (x >> 2);
    x = a1_h0(x);
    x = a1_h1(x);
    x = a1_h2(x);
    x = a1_h3(x);
    x = a1_h4(x);
    print_u32(x);
}

fn a2_h0(x: u32) -> u32 {
    x.wrapping_mul(23) ^ 11
}
fn a2_h1(x: u32) -> u32 {
    x.wrapping_mul(25) ^ 12
}
fn a2_h2(x: u32) -> u32 {
    x.wrapping_mul(27) ^ 13
}
fn a2_h3(x: u32) -> u32 {
    x.wrapping_mul(29) ^ 14
}
fn a2_h4(x: u32) -> u32 {
    x.wrapping_mul(31) ^ 15
}

fn a2_burn() {
    let mut x = black_box(3u32);
    x = x.wrapping_mul(3).wrapping_add(11) ^ (x >> 1);
    x = x.wrapping_mul(5).wrapping_add(12) ^ (x >> 2);
    x = x.wrapping_mul(7).wrapping_add(13) ^ (x >> 3);
    x = x.wrapping_mul(9).wrapping_add(14) ^ (x >> 4);
    x = x.wrapping_mul(11).wrapping_add(15) ^ (x >> 5);
    x = x.wrapping_mul(13).wrapping_add(16) ^ (x >> 6);
    x = x.wrapping_mul(15).wrapping_add(17) ^ (x >> 7);
    x = x.wrapping_mul(17).wrapping_add(18) ^ (x >> 1);
    x = x.wrapping_mul(19).wrapping_add(19) ^ (x >> 2);
    x = x.wrapping_mul(21).wrapping_add(20) ^ (x >> 3);
    x = x.wrapping_mul(23).wrapping_add(21) ^ (x >> 4);
    x = x.wrapping_mul(25).wrapping_add(22) ^ (x >> 5);
    x = x.wrapping_mul(27).wrapping_add(23) ^ (x >> 6);
    x = x.wrapping_mul(29).wrapping_add(24) ^ (x >> 7);
    x = x.wrapping_mul(31).wrapping_add(25) ^ (x >> 1);
    x = x.wrapping_mul(33).wrapping_add(26) ^ (x >> 2);
    x = a2_h0(x);
    x = a2_h1(x);
    x = a2_h2(x);
    x = a2_h3(x);
    x = a2_h4(x);
    print_u32(x);
}

fn a3_h0(x: u32) -> u32 {
    x.wrapping_mul(33) ^ 16
}
fn a3_h1(x: u32) -> u32 {
    x.wrapping_mul(35) ^ 17
}
fn a3_h2(x: u32) -> u32 {
    x.wrapping_mul(37) ^ 18
}
fn a3_h3(x: u32) -> u32 {
    x.wrapping_mul(39) ^ 19
}
fn a3_h4(x: u32) -> u32 {
    x.wrapping_mul(41) ^ 20
}

fn a3_burn() {
    let mut x = black_box(4u32);
    x = x.wrapping_mul(3).wrapping_add(11) ^ (x >> 1);
    x = x.wrapping_mul(5).wrapping_add(12) ^ (x >> 2);
    x = x.wrapping_mul(7).wrapping_add(13) ^ (x >> 3);
    x = x.wrapping_mul(9).wrapping_add(14) ^ (x >> 4);
    x = x.wrapping_mul(11).wrapping_add(15) ^ (x >> 5);
    x = x.wrapping_mul(13).wrapping_add(16) ^ (x >> 6);
    x = x.wrapping_mul(15).wrapping_add(17) ^ (x >> 7);
    x = x.wrapping_mul(17).wrapping_add(18) ^ (x >> 1);
    x = x.wrapping_mul(19).wrapping_add(19) ^ (x >> 2);
    x = x.wrapping_mul(21).wrapping_add(20) ^ (x >> 3);
    x = x.wrapping_mul(23).wrapping_add(21) ^ (x >> 4);
    x = x.wrapping_mul(25).wrapping_add(22) ^ (x >> 5);
    x = x.wrapping_mul(27).wrapping_add(23) ^ (x >> 6);
    x = x.wrapping_mul(29).wrapping_add(24) ^ (x >> 7);
    x = x.wrapping_mul(31).wrapping_add(25) ^ (x >> 1);
    x = x.wrapping_mul(33).wrapping_add(26) ^ (x >> 2);
    x = a3_h0(x);
    x = a3_h1(x);
    x = a3_h2(x);
    x = a3_h3(x);
    x = a3_h4(x);
    print_u32(x);
}

fn part() {
    let mut a = S { f: black_box(1) };
    if black_box(true) {
        a = S { f: black_box(7) };
    }
    set(&mut a, black_box(254));
    print_i32(a.f as i32);
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    a0_burn();
    a1_burn();
    a2_burn();
    a3_burn();
    part();
}
