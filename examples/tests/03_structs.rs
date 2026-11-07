#![feature(no_core, lang_items, intrinsics, rustc_attrs, decl_macro)]
#![feature(auto_traits, freeze_impls, unboxed_closures)]
#![allow(internal_features, dead_code, unused_variables)]
#![no_core]
#![no_main]

#[path = "../mini_core.rs"]
mod mini_core;
use mini_core::*;

struct Point {
    x: i32,
    y: i32,
}

impl Copy for Point {}

struct Rect {
    origin: Point,
    w: i32,
    h: i32,
}

struct Meters(f64);

struct Unit;

impl Point {
    fn sum(&self) -> i32 {
        self.x + self.y
    }

    fn shift(&mut self, dx: i32, dy: i32) {
        self.x += dx;
        self.y += dy;
    }
}

fn area(r: &Rect) -> i32 {
    r.w * r.h
}

fn grow(r: &mut Rect, by: i32) {
    r.w += by;
    r.h += by;
}

fn bump(v: &mut i32) {
    *v += 1;
}

fn read(v: &i32) -> i32 {
    *v
}

#[no_mangle]
fn rust_entry() {
    let mut p = Point { x: 3, y: 4 };
    print_i32(p.x);
    print_i32(p.y);
    print_i32(p.sum());

    // &mut self method
    p.shift(10, 20);
    print_i32(p.x);
    print_i32(p.y);
    print_i32(p.sum());

    // &mut to a field
    let fx: &mut i32 = &mut p.x;
    *fx = 100;
    print_i32(p.x);
    bump(&mut p.y);
    print_i32(p.y);

    // & and &mut to locals
    let mut n: i32 = 5;
    bump(&mut n);
    bump(&mut n);
    print_i32(n);
    print_i32(read(&n));

    // nested structs behind & / &mut
    let mut r = Rect { origin: Point { x: 0, y: 0 }, w: 6, h: 7 };
    print_i32(area(&r));
    grow(&mut r, 3);
    print_i32(area(&r));
    r.origin.shift(1, 2);
    print_i32(r.origin.x);
    print_i32(r.origin.y);

    // copy semantics: `p` stays usable
    let q = p;
    print_i32(q.x);
    print_i32(p.x + q.x);

    // tuple struct
    let m = Meters(1.5);
    print_f64(m.0);
    print_f64(m.0 * 2.0);

    // plain tuples
    let t: (i32, bool, f64) = (7, true, 2.25);
    print_i32(t.0);
    print_bool(t.1);
    print_f64(t.2);
    let mut t2 = (1, 2);
    t2.0 = 9;
    print_i32(t2.0 + t2.1);

    // nested tuple
    let nt = ((1, 2), 3);
    print_i32((nt.0).0 + (nt.0).1 + nt.1);

    // zero-sized struct
    let _u = Unit;

    print_str("03_structs ok");
}
