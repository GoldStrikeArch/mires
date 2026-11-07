#![feature(no_core, lang_items, intrinsics, rustc_attrs, decl_macro)]
#![feature(auto_traits, freeze_impls, unboxed_closures)]
#![allow(internal_features, dead_code, unused_variables)]
#![no_core]
#![no_main]

#[path = "../mini_core.rs"]
mod mini_core;
use mini_core::*;

enum Shape {
    Nothing,
    Num(i32),
    Pair(i32, i32),
    Named { w: i32, h: i32 },
}

enum Sign {
    Neg,
    Zero,
    Pos,
}

fn describe(s: &Shape) -> i32 {
    match s {
        Shape::Nothing => 0,
        Shape::Num(n) => *n,
        Shape::Pair(a, b) => *a + *b,
        Shape::Named { w, h } => *w * *h,
    }
}

fn tag(s: &Shape) -> i32 {
    match s {
        Shape::Nothing => 10,
        Shape::Num(_) => 11,
        Shape::Pair(_, _) => 12,
        Shape::Named { .. } => 13,
    }
}

fn double_payload(s: &mut Shape) {
    match s {
        Shape::Nothing => {}
        Shape::Num(n) => *n *= 2,
        Shape::Pair(a, b) => {
            *a *= 2;
            *b *= 2;
        }
        Shape::Named { w, h } => {
            *w += 1;
            *h += 1;
        }
    }
}

fn sign_of(n: i32) -> Sign {
    if n < 0 {
        Sign::Neg
    } else if n == 0 {
        Sign::Zero
    } else {
        Sign::Pos
    }
}

fn sign_value(s: Sign) -> i32 {
    match s {
        Sign::Neg => -1,
        Sign::Zero => 0,
        Sign::Pos => 1,
    }
}

#[no_mangle]
fn rust_entry() {
    let a = Shape::Nothing;
    let b = Shape::Num(7);
    let c = Shape::Pair(3, 4);
    let d = Shape::Named { w: 5, h: 6 };

    // payload reads by reference
    print_i32(describe(&a));
    print_i32(describe(&b));
    print_i32(describe(&c));
    print_i32(describe(&d));

    // discriminant dispatch
    print_i32(tag(&a));
    print_i32(tag(&b));
    print_i32(tag(&c));
    print_i32(tag(&d));

    // payload mutation through &mut
    let mut e = Shape::Num(21);
    double_payload(&mut e);
    print_i32(describe(&e));

    let mut f = Shape::Pair(1, 2);
    double_payload(&mut f);
    print_i32(describe(&f));

    let mut g = Shape::Named { w: 2, h: 3 };
    double_payload(&mut g);
    print_i32(describe(&g));

    // fieldless enum built and consumed by value
    print_i32(sign_value(sign_of(-9)));
    print_i32(sign_value(sign_of(0)));
    print_i32(sign_value(sign_of(9)));

    // match with a guard
    let h = Shape::Pair(10, 20);
    let r = match h {
        Shape::Pair(x, y) if x > y => 1,
        Shape::Pair(x, y) => x + y,
        _ => 0,
    };
    print_i32(r);

    // match by value on a named-field variant
    let i = Shape::Named { w: 4, h: 4 };
    let boxed_area = match i {
        Shape::Named { w, h } => w * h,
        _ => 0,
    };
    print_i32(boxed_area);

    print_str("04_enums ok");
}
