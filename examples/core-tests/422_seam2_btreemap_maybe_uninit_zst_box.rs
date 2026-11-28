//! Seam 2 regression: unified addressable-place rule & ZST deref lowering.
//! Exercises BTreeMap/BTreeSet drop (`Handle::drop_key_val`), `MaybeUninit::write_filled`
//! (including ZST), `slice::fill_with` on ZSTs, `Box::<()>::new_uninit()`, and raw pointers
//! to direct/transparent-wrapped locals (`MaybeUninit`, `ManuallyDrop`, `UnsafeCell`).

#![no_std]
#![no_main]
#![feature(maybe_uninit_fill)]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::{Cell, UnsafeCell};
use core::mem::{ManuallyDrop, MaybeUninit};
use core::ptr::{self, NonNull};
use prelude::*;

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // 1. BTreeMap<String, Vec<i32>> and BTreeSet<i32> / BTreeSet<String> drop
    let mut map: BTreeMap<String, Vec<i32>> = BTreeMap::new();
    for i in 0..25 {
        let mut key = String::from("k");
        key.push(char::from(b'a' + (i as u8)));
        map.insert(key, vec![i, i * 2]);
    }
    print_usize(map.len());
    print_bool(map.get("kh") == Some(&vec![7, 14]));
    drop(map);

    let mut set: BTreeSet<i32> = BTreeSet::new();
    for i in (0..30).rev() {
        set.insert(i);
    }
    print_usize(set.len());
    print_bool(set.remove(&15));
    drop(set);

    let mut sset: BTreeSet<String> = BTreeSet::new();
    for i in 0..10 {
        let mut s = String::from("s");
        s.push(char::from(b'0' + (i as u8)));
        sset.insert(s);
    }
    print_usize(sset.len());
    drop(sset);

    // 2. MaybeUninit::write_filled (scalar, String, and ZST)
    let mut buf: [MaybeUninit<u32>; 4] = [const { MaybeUninit::uninit() }; 4];
    let filled = buf.write_filled(42);
    print_u32(filled[0] + filled[3]);

    let mut sbuf: [MaybeUninit<String>; 2] = [const { MaybeUninit::uninit() }; 2];
    let sfilled = sbuf.write_filled(String::from("hi"));
    print_bool(sfilled[0] == "hi" && sfilled[1] == "hi");
    // SAFETY: `write_filled` initialized every element of `sfilled`.
    unsafe {
        ptr::drop_in_place(sfilled as *mut [String]);
    }

    let mut zbuf: [MaybeUninit<()>; 4] = [const { MaybeUninit::uninit() }; 4];
    let zfilled = zbuf.write_filled(());
    print_usize(zfilled.len());

    let mut zslice: [(); 5] = [(); 5];
    let mut fills = 0usize;
    zslice.fill_with(|| {
        fills += 1;
    });
    print_usize(fills);

    // 3. Zero-sized Box::new_uninit / new_uninit_slice
    let zb = Box::<()>::new_uninit();
    print_bool(&*zb as *const MaybeUninit<()> == NonNull::dangling().as_ptr());
    // SAFETY: `()` has no uninitialized bytes.
    let zinit = unsafe { zb.assume_init() };
    print_bool(*zinit == ());

    let zbs = Box::<[()]>::new_uninit_slice(5);
    print_usize(zbs.len());
    // SAFETY: `[()]` has no uninitialized bytes.
    let zbs_init = unsafe { zbs.assume_init() };
    print_usize(zbs_init.len());

    let empty_bs = Box::<[String]>::new_uninit_slice(0);
    print_usize(empty_bs.len());

    // 4. Address-taken transparent-wrapped direct locals
    let mut mu = MaybeUninit::<u32>::uninit();
    let p_mu = mu.as_mut_ptr();
    // SAFETY: `p_mu` points to stack storage of `mu`.
    unsafe {
        p_mu.write(123);
        print_u32(mu.assume_init());
    }

    let mut md = ManuallyDrop::new(77_u32);
    let p_md: *mut u32 = &mut *md;
    // SAFETY: `p_md` points to live `md`.
    unsafe {
        *p_md += 11;
    }
    print_u32(*md);

    let uc = UnsafeCell::new(50_u32);
    // SAFETY: exclusive access to `uc`.
    unsafe {
        *uc.get() += 5;
    }
    print_u32(uc.into_inner());

    let c = Cell::new(9_u32);
    c.set(c.get() + 1);
    print_u32(c.get());
}
