#![feature(no_core, lang_items, intrinsics, rustc_attrs, decl_macro)]
#![feature(auto_traits, freeze_impls, unboxed_closures)]
#![allow(internal_features, dead_code, unused_variables)]
#![no_core]
#![no_main]

#[path = "../mini_core.rs"]
mod mini_core;
use mini_core::*;

fn classify(n: i32) -> i32 {
    if n < 0 {
        -1
    } else if n == 0 {
        0
    } else if n < 10 {
        1
    } else {
        2
    }
}

fn match_small(n: i32) -> i32 {
    match n {
        0 => 100,
        1 | 2 => 200,
        3..=5 => 300,
        _ => 400,
    }
}

fn nested(a: i32, b: i32) -> i32 {
    match a {
        0 => match b {
            0 => 1,
            1 => 2,
            _ => 3,
        },
        1 => match b {
            0 => 4,
            _ => 5,
        },
        _ => 6,
    }
}

#[no_mangle]
fn rust_entry() {
    // if / else-if / else
    print_i32(classify(-5));
    print_i32(classify(0));
    print_i32(classify(7));
    print_i32(classify(50));

    // match with literal, or- and range-patterns
    print_i32(match_small(0));
    print_i32(match_small(1));
    print_i32(match_small(2));
    print_i32(match_small(4));
    print_i32(match_small(9));

    // nested match
    print_i32(nested(0, 0));
    print_i32(nested(0, 1));
    print_i32(nested(0, 9));
    print_i32(nested(1, 0));
    print_i32(nested(1, 5));
    print_i32(nested(8, 8));

    // while
    let mut i: i32 = 0;
    let mut sum: i32 = 0;
    while i < 10 {
        sum += i;
        i += 1;
    }
    print_i32(sum);

    // while with an early break
    let mut j: i32 = 0;
    while j < 100 {
        if j == 13 {
            break;
        }
        j += 1;
    }
    print_i32(j);

    // loop with continue and break
    let mut k: i32 = 0;
    let mut odd_sum: i32 = 0;
    loop {
        k += 1;
        if k > 20 {
            break;
        }
        if k % 2 == 0 {
            continue;
        }
        odd_sum += k;
    }
    print_i32(odd_sum);

    // loop that breaks with a value
    let mut m: i32 = 1;
    let found = loop {
        m *= 2;
        if m > 100 {
            break m;
        }
    };
    print_i32(found);

    // labelled loops: `continue 'outer` and `break 'outer`
    let mut hits: i32 = 0;
    let mut r: i32 = 0;
    'outer: while r < 5 {
        let mut c: i32 = 0;
        while c < 5 {
            if c == 3 {
                r += 1;
                continue 'outer;
            }
            if r == 3 && c == 2 {
                break 'outer;
            }
            hits += 1;
            c += 1;
        }
        r += 1;
    }
    print_i32(hits);
    print_i32(r);

    // match used as an expression
    let bucket = match classify(42) {
        0 => 1000,
        1 => 2000,
        2 => 3000,
        _ => 4000,
    };
    print_i32(bucket);

    print_str("02_control ok");
}
