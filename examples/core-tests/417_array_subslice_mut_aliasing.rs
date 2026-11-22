//! Mutable & aliasing array `Subslice` (`[_, middle @ .., _]`) across `&mut [T; N]` params,
//! local arrays, struct fields, 2D arrays, `Box<[T; N]>`, nested rest patterns, prefix/suffix
//! patterns, slice methods, whole-subslice overwrite, `mem::replace`/`swap`, raw pointers,
//! single evaluation, empty `[u32; 0]` subslices, ZST `[(); 6]` subslices, and pointer identity.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::boxed::Box;
use core::hint::black_box;
use core::mem;
use core::ptr;

#[inline(never)]
fn update_middle(array: &mut [u32; 6]) {
    let [_, middle @ .., _] = array;
    middle[black_box(1)] = 99;
}

#[inline(never)]
fn overwrite_four(target: &mut [u32; 4], replacement: [u32; 4]) {
    *target = replacement;
}

#[inline(never)]
fn nested_update(array: &mut [u32; 6]) {
    let [_, middle @ .., _] = array;
    let [_, inner @ .., _] = middle;
    inner[black_box(0)] = 701;
    inner[black_box(1)] = 702;
}

#[inline(never)]
fn pick_array<'a>(array: &'a mut [u32; 6], calls: &mut u32) -> &'a mut [u32; 6] {
    *calls += 1;
    array
}

struct Holder {
    tag: u32,
    data: [u32; 6],
}

#[inline(never)]
fn test_exact_repro_and_local() {
    // 1. Exact `array_middle.rs` shape
    let mut array = black_box([10u32, 20, 30, 40, 50, 60]);
    update_middle(&mut array);
    print_u32(array[2]);

    // 2. Single-use local array under js-opt / inline+Oz
    let mut local = [10u32, 20, 30, 40];
    let [_, m @ .., _] = &mut local;
    m[0] = 99;
    m[black_box(1)] = 88;
    print_u32(local[1]);
    print_u32(local[2]);

    // 3. Nested rest patterns (middle -> inner)
    let mut nested = black_box([1u32, 2, 3, 4, 5, 6]);
    nested_update(&mut nested);
    print_u32(nested[1]);
    print_u32(nested[2]);
    print_u32(nested[3]);
    print_u32(nested[4]);

    // 4. Prefix, suffix, and full rest patterns
    let mut ps = black_box([10u32, 20, 30, 40, 50, 60]);
    {
        let [head @ .., last] = &mut ps;
        head[black_box(0)] = 111;
        head[4] = 555;
        *last = 666;
    }
    print_u32(ps[0]);
    print_u32(ps[4]);
    print_u32(ps[5]);
    {
        let [first, tail @ ..] = &mut ps;
        *first = 123;
        tail[black_box(1)] = 333;
    }
    print_u32(ps[0]);
    print_u32(ps[2]);
}

#[inline(never)]
fn test_containers_and_slice_methods() {
    // 5. Struct field array
    let mut holder = Holder {
        tag: 7,
        data: black_box([1u32, 2, 3, 4, 5, 6]),
    };
    {
        let [_, mid @ .., _] = &mut holder.data;
        mid[black_box(2)] = 44;
    }
    print_u32(holder.tag);
    print_u32(holder.data[3]);

    // 6. Nested 2D array (outer subslice + inner row subslice)
    let mut grid = black_box([
        [1u32, 2, 3, 4, 5],
        [10, 20, 30, 40, 50],
        [100, 200, 300, 400, 500],
        [1000, 2000, 3000, 4000, 5000],
    ]);
    {
        let [_, rows @ .., _] = &mut grid;
        let [_, inner @ .., _] = &mut rows[black_box(1)];
        inner[black_box(1)] = 333;
    }
    print_u32(grid[2][2]);

    // 7. Heap Box<[u32; 6]>
    let mut boxed = Box::new(black_box([10u32, 20, 30, 40, 50, 60]));
    {
        let [_, mid @ .., _] = &mut *boxed;
        mid[black_box(1)] = 309;
        mid.swap(0, 3);
    }
    print_u32(boxed[1]);
    print_u32(boxed[2]);
    print_u32(boxed[4]);

    // 8. Slice methods: as_mut_slice, reverse, swap
    let mut arr = black_box([1u32, 2, 3, 4, 5, 6]);
    {
        let [_, mid @ .., _] = &mut arr;
        let s: &mut [u32] = mid.as_mut_slice();
        s[1] = 30;
        mid.reverse();
        mid.swap(0, 2);
    }
    // Original mid was [2, 3, 4, 5] -> s[1]=30 => [2, 30, 4, 5] -> reverse => [5, 4, 30, 2]
    // -> swap(0, 2) => [30, 4, 5, 2]
    print_u32(arr[0]);
    print_u32(arr[1]);
    print_u32(arr[2]);
    print_u32(arr[3]);
    print_u32(arr[4]);
    print_u32(arr[5]);
}

#[inline(never)]
fn test_overwrite_replace_swap_and_raw() {
    // 9. Whole-subslice overwrite (both direct and via opaque &mut [u32; 4] callee)
    let mut arr = black_box([1u32, 2, 3, 4, 5, 6]);
    {
        let [_, mid @ .., _] = &mut arr;
        *mid = [101, 102, 103, 104];
    }
    print_u32(arr[0]);
    print_u32(arr[1]);
    print_u32(arr[4]);
    print_u32(arr[5]);
    {
        let [_, mid @ .., _] = &mut arr;
        overwrite_four(mid, [201, 202, 203, 204]);
        // Subsequent write through same view must still alias `arr`!
        mid[black_box(2)] = 299;
    }
    print_u32(arr[1]);
    print_u32(arr[3]);
    print_u32(arr[4]);

    // 10. core::mem::replace and core::mem::swap on subslice refs
    let mut arr2 = black_box([10u32, 20, 30, 40, 50, 60]);
    let old = {
        let [_, mid @ .., _] = &mut arr2;
        mem::replace(mid, [91, 92, 93, 94])
    };
    print_u32(old[0]);
    print_u32(old[3]);
    print_u32(arr2[1]);
    print_u32(arr2[4]);

    let mut other = black_box([500u32, 501, 502, 503]);
    {
        let [_, mid @ .., _] = &mut arr2;
        mem::swap(mid, &mut other);
    }
    print_u32(arr2[1]);
    print_u32(arr2[4]);
    print_u32(other[0]);
    print_u32(other[3]);

    // 11. Raw pointer `&raw mut (*middle)` and single-eval effectful base
    let mut arr3 = black_box([1u32, 2, 3, 4, 5, 6]);
    let mut calls = 0u32;
    {
        let [_, mid @ .., _] = pick_array(&mut arr3, &mut calls);
        let raw: *mut [u32; 4] = &raw mut (*mid);
        unsafe {
            (*raw)[black_box(1)] = 888;
            let elem_ptr: *const u32 = raw as *const u32;
            print_bool(ptr::eq(elem_ptr, &arr3[1]));
            print_bool(ptr::eq(elem_ptr.add(2), &arr3[3]));
        }
    }
    print_u32(calls);
    print_u32(arr3[2]);
}

#[inline(never)]
fn test_shared_identity_empty_and_zst() {
    // 12. Shared borrow reading, slice/iter, and pointer identity
    let arr = black_box([10u32, 20, 30, 40, 50, 60]);
    let [_, mid @ .., _] = &arr;
    let [_, inner @ .., _] = mid;
    let [head @ .., _] = &arr;
    let [_, tail @ ..] = &arr;

    print_u32(mid[black_box(1)]);
    print_u32(mid.as_slice()[2]);
    let mut sum = 0u32;
    for &x in mid.iter() {
        sum += x;
    }
    print_u32(sum);

    print_bool(ptr::eq(mid.as_ptr(), &arr[1]));
    print_bool(ptr::eq(head.as_ptr(), &arr[0]));
    print_bool(ptr::eq(tail.as_ptr(), &arr[1]));
    print_bool(ptr::eq(inner.as_ptr(), &arr[2]));
    print_bool(ptr::eq(&mid[2], &arr[3]));
    print_bool(ptr::eq(
        mid as *const [u32; 4] as *const (),
        &arr[1] as *const u32 as *const (),
    ));
    print_bool(ptr::eq(
        head as *const [u32; 5] as *const (),
        &arr as *const [u32; 6] as *const (),
    ));
    print_bool(mid.as_ptr() < (&arr[2] as *const u32));
    print_bool((&arr[0] as *const u32) < mid.as_ptr());

    // 13. Empty array subslice `[u32; 0]`
    let mut arr3 = black_box([7u32, 8, 9]);
    {
        let [a, b, mid0 @ .., c] = &mut arr3;
        print_u32(*a);
        print_u32(*b);
        print_u32(*c);
        print_usize(mid0.len());
        print_usize(mid0.as_slice().len());
        *mid0 = [];
        print_bool(ptr::eq(mid0.as_ptr(), &arr3[2]));
    }
    {
        let [head0 @ .., _, _, _] = &arr3;
        print_usize(head0.len());
        print_bool(ptr::eq(head0.as_ptr(), &arr3[0]));
    }

    // 14. ZST array subslice `[(); 6]`
    let mut zst = [(); 6];
    let [_, zmid @ .., _] = &mut zst;
    print_usize(zmid.len());
    let [_, zval @ .., _] = zst;
    print_usize(zval.len());
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    test_exact_repro_and_local();
    test_containers_and_slice_methods();
    test_overwrite_replace_swap_and_raw();
    test_shared_identity_empty_and_zst();
}
