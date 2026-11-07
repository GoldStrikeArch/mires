#![feature(no_core, lang_items, intrinsics, rustc_attrs, decl_macro)]
#![feature(auto_traits, freeze_impls, unboxed_closures)]
#![allow(internal_features, dead_code, unused_variables)]
#![no_core]
#![no_main]

#[path = "../mini_core.rs"]
mod mini_core;
use mini_core::*;

trait Shape {
    // Required: every impl provides its own vtable slot.
    fn area(&self) -> i32;

    // Default, never overridden: the vtable slot points at the trait's own body.
    fn sides(&self) -> i32 {
        4
    }

    // Default that calls back through `self`, so the inner call is itself virtual
    // when reached through `dyn`.
    fn scaled(&self) -> i32 {
        self.area() * 2
    }

    // Default that one impl overrides and another does not.
    fn tag(&self) -> i32 {
        0
    }

    fn grow(&mut self, by: i32);
}

struct Square {
    side: i32,
}

struct Rect {
    w: i32,
    h: i32,
}

impl Shape for Square {
    fn area(&self) -> i32 {
        self.side * self.side
    }

    fn grow(&mut self, by: i32) {
        self.side += by;
    }
}

impl Shape for Rect {
    fn area(&self) -> i32 {
        self.w * self.h
    }

    fn sides(&self) -> i32 {
        44
    }

    fn tag(&self) -> i32 {
        7
    }

    fn grow(&mut self, by: i32) {
        self.w += by;
        self.h += by;
    }
}

// One call site, two concrete types.
fn describe(s: &dyn Shape) -> i32 {
    s.area() + s.sides() + s.tag()
}

// A `&dyn` stored in a struct field.
struct Holder<'a> {
    shape: &'a dyn Shape,
    weight: i32,
}

impl<'a> Holder<'a> {
    fn total(&self) -> i32 {
        self.shape.area() * self.weight
    }
}

#[no_mangle]
fn rust_entry() {
    let sq = Square { side: 3 };
    let rc = Rect { w: 2, h: 5 };

    // Coercion to `&dyn Trait` and a required-method call.
    let a: &dyn Shape = &sq;
    print_i32(a.area());

    // Default method through `dyn` (not overridden).
    print_i32(a.sides());

    // Default method that re-dispatches through `self`.
    print_i32(a.scaled());

    // Default `tag` not overridden by `Square`.
    print_i32(a.tag());

    let b: &dyn Shape = &rc;
    print_i32(b.area());

    // Overridden method through `dyn`.
    print_i32(b.sides());
    print_i32(b.tag());
    print_i32(b.scaled());

    // The same call site reached with both concrete types.
    print_i32(describe(a));
    print_i32(describe(b));
    print_i32(describe(&sq));
    print_i32(describe(&rc));

    // `&dyn` stored in a struct field.
    let h = Holder { shape: a, weight: 10 };
    print_i32(h.total());
    let h2 = Holder { shape: b, weight: 3 };
    print_i32(h2.total());
    print_i32(h2.shape.sides());

    // `&mut dyn` receiver mutating the concrete value behind it.
    let mut sq2 = Square { side: 4 };
    {
        let m: &mut dyn Shape = &mut sq2;
        m.grow(2);
        print_i32(m.area());
    }
    print_i32(sq2.side);

    // Re-borrowing the same concrete value through two separate coercions.
    let c: &dyn Shape = &sq2;
    print_i32(describe(c));

    print_str("10_dyn ok");
}
