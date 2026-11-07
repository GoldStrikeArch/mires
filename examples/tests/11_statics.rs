#![feature(no_core, lang_items, intrinsics, rustc_attrs, decl_macro)]
#![feature(auto_traits, freeze_impls, unboxed_closures)]
#![allow(internal_features, dead_code, unused_variables)]
#![no_core]
#![no_main]

#[path = "../mini_core.rs"]
mod mini_core;
use mini_core::*;

static LIMIT: i32 = 42;
static NEGATIVE: i32 = -7;
static FLAG: bool = true;
static RATIO: f64 = 1.5;

static mut COUNTER: i32 = 0;

static ARR: [i32; 3] = [10, 20, 30];
static NESTED: [i32; 2] = [LIMIT, 5];

struct Config {
    width: i32,
    height: i32,
    scale: f64,
}

unsafe impl Sync for Config {}

static CONFIG: Config = Config { width: 640, height: 480, scale: 2.0 };

static NAME: &str = "topcoat";
static EMPTY: &str = "";

fn bump(by: i32) -> i32 {
    unsafe {
        COUNTER += by;
        COUNTER
    }
}

fn read_arr(i: usize) -> i32 {
    ARR[i]
}

#[no_mangle]
fn rust_entry() {
    // Scalar statics.
    print_i32(LIMIT);
    print_i32(NEGATIVE);
    print_bool(FLAG);
    print_f64(RATIO);

    // `static mut` mutated through `unsafe` and read back.
    print_i32(bump(1));
    print_i32(bump(2));
    unsafe {
        COUNTER += 10;
        print_i32(COUNTER);
    }

    // Array statics, constant and computed indices.
    print_i32(ARR[0]);
    print_i32(ARR[2]);
    print_i32(read_arr(1));
    print_i32(NESTED[0]);
    print_i32(NESTED[1]);

    // Struct static, field reads.
    print_i32(CONFIG.width);
    print_i32(CONFIG.height);
    print_f64(CONFIG.scale);
    print_i32(CONFIG.width + CONFIG.height);

    // `&'static str` statics.
    print_str(NAME);
    print_str(EMPTY);

    // A static read through a reference to it.
    let r: &i32 = &LIMIT;
    print_i32(*r);
    let a: &[i32; 3] = &ARR;
    print_i32(a[1]);

    print_str("11_statics ok");
}
