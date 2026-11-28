//! Seam 4 regression: unified Box / dyn Any / NativeVec / raw dealloc contract & const `vec![]`.
//!
//! Exercises:
//! 1. `Box<dyn Any>` over `Vec<u8>`, `String`, scalar, aggregate, and ZST (`()`) drop.
//! 2. `Box::<MaybeUninit<Shared>>::new_uninit()` + `ptr::write` + `assume_init` + drop
//!    (the `bytes::Bytes::drop` `_boxes` registration pattern).
//! 3. `Box::into_raw` -> `alloc::alloc::dealloc` and `alloc` -> `Box::from_raw` -> drop.
//! 4. `const` `vec![...]` (`patches/0023-native-vec.patch` `const fn rcgjs_vec_from_array`).

#![no_std]
#![no_main]
#![feature(const_heap, const_destruct, const_index, const_trait_impl)]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;

use alloc::alloc::{Layout, alloc, dealloc};
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::any::Any;
use core::mem::MaybeUninit;
use core::ptr;
use core::sync::atomic::{AtomicUsize, Ordering};
use prelude::*;

struct Pair {
    a: Vec<i32>,
    b: String,
}

struct Shared {
    vec: Vec<u8>,
    ref_cnt: AtomicUsize,
}

const CONST_VEC_SUM: u32 = {
    let v = vec![10_u32, 20, 30, 40];
    let mut sum = 0;
    let mut i = 0;
    while i < v.len() {
        sum += v[i];
        i += 1;
    }
    sum
};

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // 1. Box<dyn Any> over Vec<u8>, String, i32, struct, ZST
    let b_vec: Box<dyn Any> = Box::new(vec![1_u8, 2, 3, 4]);
    print_bool(b_vec.downcast_ref::<Vec<u8>>() == Some(&vec![1_u8, 2, 3, 4]));
    drop(b_vec);

    let b_str: Box<dyn Any> = Box::new(String::from("dyn-heap"));
    print_bool(b_str.downcast_ref::<String>().map(String::as_str) == Some("dyn-heap"));
    drop(b_str);

    let b_int: Box<dyn Any> = Box::new(12345_i32);
    print_bool(b_int.downcast_ref::<i32>() == Some(&12345));
    drop(b_int);

    let b_pair: Box<dyn Any> = Box::new(Pair {
        a: vec![10, 20],
        b: String::from("ok"),
    });
    print_bool(
        b_pair
            .downcast_ref::<Pair>()
            .is_some_and(|p| p.a == vec![10, 20] && p.b == "ok"),
    );
    drop(b_pair);

    let b_zst: Box<dyn Any> = Box::new(());
    print_bool(b_zst.is::<()>());
    drop(b_zst);

    // 2. Box::<MaybeUninit<Shared>>::new_uninit() -> ptr::write -> Box::into_raw -> Box::from_raw -> drop
    let mut uninit_box: Box<MaybeUninit<Shared>> = Box::new_uninit();
    // SAFETY: `uninit_box` points to valid uninitialized storage for `Shared`.
    unsafe {
        ptr::write(
            uninit_box.as_mut_ptr(),
            Shared {
                vec: vec![5, 6, 7],
                ref_cnt: AtomicUsize::new(1),
            },
        );
        let raw_shared: *mut Shared = Box::into_raw(uninit_box.assume_init());
        let recovered = Box::from_raw(raw_shared);
        print_usize(recovered.vec.len());
        print_usize(recovered.ref_cnt.load(Ordering::Relaxed));
        drop(recovered);
    }

    // 3. Box::into_raw -> manual dealloc & manual alloc -> Box::from_raw
    let raw = Box::into_raw(Box::new([10_u32, 20, 30]));
    // SAFETY: `raw` was allocated with `Layout::new::<[u32; 3]>()`.
    unsafe {
        print_u32((*raw)[0] + (*raw)[2]);
        dealloc(raw.cast::<u8>(), Layout::new::<[u32; 3]>());
    }

    // SAFETY: allocates properly aligned storage for `Vec<u8>` and transfers ownership to `Box`.
    unsafe {
        let layout = Layout::new::<Vec<u8>>();
        let p = alloc(layout).cast::<Vec<u8>>();
        p.write(vec![7, 8, 9]);
        let b = Box::from_raw(p);
        print_usize(b.len());
        drop(b);
    }

    // 4. const vec![] + runtime vec![]
    print_u32(CONST_VEC_SUM);
    let mut rt = vec![1_u32, 2, 3];
    rt.push(4);
    print_usize(rt.len());
}
