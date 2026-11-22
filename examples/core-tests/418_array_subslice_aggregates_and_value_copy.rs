//! Aggregate element array `Subslice` (`[Pair; 6]`, `[String; 5]`, `[Cell<u32>; 6]`, `[&[u32]; 6]`)
//! and owned by-value copy isolation (`let [_, mid @ .., _] = arr;`).

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::string::String;
use core::cell::Cell;
use core::hint::black_box;
use core::mem;
use core::ptr;

#[derive(Copy, Clone)]
struct Pair {
    x: i32,
    y: i32,
}

#[inline(never)]
fn mutate_pairs(arr: &mut [Pair; 6]) {
    let [_, middle @ .., _] = arr;
    middle[black_box(1)].x = 77;
    {
        let [_, inner @ .., _] = middle;
        inner[black_box(0)].y = 88;
    }
    middle[2] = Pair { x: 500, y: 600 };
}

#[inline(never)]
fn overwrite_pairs(target: &mut [Pair; 4], replacement: [Pair; 4]) {
    *target = replacement;
}

#[inline(never)]
fn test_pair_array_subslice_and_value_copy() {
    let mut arr = black_box([
        Pair { x: 1, y: 2 },
        Pair { x: 3, y: 4 },
        Pair { x: 5, y: 6 },
        Pair { x: 7, y: 8 },
        Pair { x: 9, y: 10 },
        Pair { x: 11, y: 12 },
    ]);

    mutate_pairs(&mut arr);
    // arr[2] had .x = 77 and inner[0].y = 88; arr[3] overwritten with (500, 600)
    print_i32(arr[2].x);
    print_i32(arr[2].y);
    print_i32(arr[3].x);
    print_i32(arr[3].y);

    // Pointer identity and slice methods on aggregate array subslice
    {
        let [_, middle @ .., _] = &arr;
        print_bool(ptr::eq(middle.as_ptr(), ptr::addr_of!(arr[1])));
        print_bool(ptr::eq(ptr::addr_of!(middle[1]), ptr::addr_of!(arr[2])));
        print_bool(ptr::eq(&middle[1], &arr[2]));
    }
    {
        let [_, middle @ .., _] = &mut arr;
        middle.swap(0, 3);
    }
    print_i32(arr[1].x);
    print_i32(arr[4].x);

    // Whole-subslice overwrite and mem::replace on [Pair; 4]
    {
        let [_, middle @ .., _] = &mut arr;
        overwrite_pairs(
            middle,
            [
                Pair { x: 10, y: 11 },
                Pair { x: 20, y: 21 },
                Pair { x: 30, y: 31 },
                Pair { x: 40, y: 41 },
            ],
        );
        middle[black_box(1)].x = 25;
    }
    print_i32(arr[1].x);
    print_i32(arr[2].x);
    print_i32(arr[4].y);

    let replaced = {
        let [_, middle @ .., _] = &mut arr;
        mem::replace(
            middle,
            [
                Pair { x: 100, y: 101 },
                Pair { x: 200, y: 201 },
                Pair { x: 300, y: 301 },
                Pair { x: 400, y: 401 },
            ],
        )
    };
    print_i32(replaced[0].x);
    print_i32(replaced[1].x);
    print_i32(arr[1].x);
    print_i32(arr[2].x);

    // Owned by-value copy via pattern: mutating `mid_copy` must NOT mutate `arr`!
    let [_, mut mid_copy @ .., _] = arr;
    mid_copy[0].x = 999;
    mid_copy[1] = Pair { x: 888, y: 777 };
    print_i32(mid_copy[0].x);
    print_i32(mid_copy[1].x);
    print_i32(arr[1].x);
    print_i32(arr[2].x);

    // Owned by-value copy via deref of subslice reference: must NOT mutate `arr`!
    let mut mid_copy2: [Pair; 4] = {
        let [_, middle @ .., _] = &arr;
        *middle
    };
    mid_copy2[0].x = 1234;
    print_i32(mid_copy2[0].x);
    print_i32(arr[1].x);
}

#[inline(never)]
fn test_string_array_subslice() {
    let mut words = [
        String::from("zero"),
        String::from("one"),
        String::from("two"),
        String::from("three"),
        String::from("four"),
    ];
    {
        let [_, middle @ .., _] = &mut words;
        middle[0].push_str("!");
        middle[black_box(1)].push_str("?");
        middle[2] = String::from("replaced");
    }
    print_str(&words[1]);
    print_str(&words[2]);
    print_str(&words[3]);

    // Whole-subslice overwrite on [String; 3]
    {
        let [_, middle @ .., _] = &mut words;
        *middle = [
            String::from("n1"),
            String::from("n2"),
            String::from("n3"),
        ];
    }
    print_str(&words[0]);
    print_str(&words[1]);
    print_str(&words[2]);
    print_str(&words[3]);
    print_str(&words[4]);

    // mem::replace on [String; 3]
    let old = {
        let [_, middle @ .., _] = &mut words;
        mem::replace(
            middle,
            [
                String::from("r1"),
                String::from("r2"),
                String::from("r3"),
            ],
        )
    };
    print_str(&old[0]);
    print_str(&old[2]);
    print_str(&words[1]);
    print_str(&words[3]);

    // Owned by-value move of [String; 3] subslice
    let [first, mut moved_mid @ .., last] = words;
    moved_mid[0].push_str("+");
    print_str(&first);
    print_str(&moved_mid[0]);
    print_str(&moved_mid[2]);
    print_str(&last);
}

#[inline(never)]
fn test_cell_and_ref_elements() {
    // Shared borrow with Cell elements (interior mutability through shared subslice)
    let cells = [
        Cell::new(10u32),
        Cell::new(20),
        Cell::new(30),
        Cell::new(40),
        Cell::new(50),
        Cell::new(60),
    ];
    let [_, mid @ .., _] = &cells;
    mid[black_box(1)].set(42);
    print_u32(cells[2].get());
    let [_, inner @ .., _] = mid;
    inner[black_box(0)].set(99);
    print_u32(cells[2].get());

    // Array of non-indirect fat slice references: overwriting subslice must rebind references
    // without mutating previously copied slice references.
    let a: &[u32] = &[1, 2];
    let b: &[u32] = &[9, 8];
    let mut refs: [&[u32]; 6] = [a, a, a, a, a, a];
    let saved = refs[1];
    {
        let [_, mid @ .., _] = &mut refs;
        *mid = [b, b, b, b];
    }
    print_u32(saved[0]);
    print_u32(refs[0][0]);
    print_u32(refs[1][0]);
    print_u32(refs[4][0]);
    print_u32(refs[5][0]);
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    test_pair_array_subslice_and_value_copy();
    test_string_array_subslice();
    test_cell_and_ref_elements();
}
