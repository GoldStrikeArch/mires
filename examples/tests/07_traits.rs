#![feature(no_core, lang_items, intrinsics, rustc_attrs, decl_macro)]
#![feature(auto_traits, freeze_impls, unboxed_closures)]
#![allow(internal_features, dead_code, unused_variables)]
#![no_core]
#![no_main]

#[path = "../mini_core.rs"]
mod mini_core;
use mini_core::*;

trait Shape {
    fn area(&self) -> i32;
    fn name(&self) -> &'static str;

    // default method, overridden by `Rect`
    fn describe(&self) -> i32 {
        self.area() * 2
    }

    // default method calling another required method, never overridden
    fn scaled(&self, k: i32) -> i32 {
        self.area() * k
    }
}

struct Square {
    side: i32,
}

struct Rect {
    w: i32,
    h: i32,
}

struct Dot;

impl Shape for Square {
    fn area(&self) -> i32 {
        self.side * self.side
    }

    fn name(&self) -> &'static str {
        "square"
    }
}

impl Shape for Rect {
    fn area(&self) -> i32 {
        self.w * self.h
    }

    fn name(&self) -> &'static str {
        "rect"
    }

    fn describe(&self) -> i32 {
        self.area() + 1000
    }
}

impl Shape for Dot {
    fn area(&self) -> i32 {
        0
    }

    fn name(&self) -> &'static str {
        "dot"
    }
}

// associated const with a default-using method
trait Tagged {
    const TAG: i32;

    fn tag(&self) -> i32 {
        Self::TAG
    }
}

impl Tagged for Square {
    const TAG: i32 = 1;
}

impl Tagged for Rect {
    const TAG: i32 = 2;

    fn tag(&self) -> i32 {
        Self::TAG * 100
    }
}

// associated type
trait Container {
    type Item;

    fn get(&self) -> Self::Item;
}

struct IntBox {
    v: i32,
}

struct FloatBox {
    v: f64,
}

impl Container for IntBox {
    type Item = i32;

    fn get(&self) -> i32 {
        self.v
    }
}

impl Container for FloatBox {
    type Item = f64;

    fn get(&self) -> f64 {
        self.v
    }
}

// trait implemented for a primitive
trait Doubler {
    fn double(&self) -> i32;
}

impl Doubler for i32 {
    fn double(&self) -> i32 {
        *self * 2
    }
}

impl Doubler for bool {
    fn double(&self) -> i32 {
        if *self { 2 } else { 0 }
    }
}

// static dispatch through generic bounds
fn report<S: Shape>(s: &S) {
    print_str(s.name());
    print_i32(s.area());
    print_i32(s.describe());
    print_i32(s.scaled(3));
}

fn total_area<A: Shape, B: Shape>(a: &A, b: &B) -> i32 {
    a.area() + b.area()
}

fn unbox<C: Container>(c: &C) -> C::Item {
    c.get()
}

fn double_all<D: Doubler>(d: &D) -> i32 {
    d.double() + d.double()
}

#[no_mangle]
fn rust_entry() {
    let sq = Square { side: 5 };
    let rc = Rect { w: 3, h: 4 };
    let dt = Dot;

    report(&sq);
    report(&rc);
    report(&dt);

    print_i32(total_area(&sq, &rc));
    print_i32(total_area(&dt, &dt));

    // associated consts
    print_i32(sq.tag());
    print_i32(rc.tag());
    print_i32(<Square as Tagged>::TAG);

    // associated types
    print_i32(unbox(&IntBox { v: 9 }));
    print_f64(unbox(&FloatBox { v: 1.25 }));

    // trait impls on primitives
    let five: i32 = 5;
    print_i32(five.double());
    let neg: i32 = -3;
    print_i32(neg.double());
    print_i32(double_all(&five));
    print_i32(double_all(&true));
    print_i32(double_all(&false));

    // fully qualified call
    print_i32(Shape::area(&sq));
    print_i32(<Rect as Shape>::describe(&rc));

    print_str("07_traits ok");
}
