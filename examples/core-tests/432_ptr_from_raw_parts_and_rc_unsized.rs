//! Regression test for generalized `pointer_from_parts` (`dyn Trait` & unsized-tail structs)
//! and `Rc::allocate_for_ptr_in` (`Rc::from(&[T])`, `Rc::from(Vec<T>)`, `Arc::from(&[T])`).

#![no_std]
#![no_main]
#![feature(ptr_metadata)]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::rc::Rc;
use alloc::sync::Arc;
use alloc::vec;
use core::ptr;

trait Greet {
    fn score(&self) -> i32;
}

struct Person(i32);
impl Greet for Person {
    fn score(&self) -> i32 {
        self.0 * 3
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // `ptr::from_raw_parts` on `dyn Trait`
    let p = Person(14);
    let dyn_ref: &dyn Greet = &p;
    let (data, meta) = ptr::from_ref(dyn_ref).to_raw_parts();
    let rebuilt: *const dyn Greet = ptr::from_raw_parts(data, meta);
    print_i32(unsafe { (&*rebuilt).score() });

    // `Rc::from(&[T])` (const/stack slice -> `allocate_for_ptr_in`)
    let rc_const: Rc<[u32]> = Rc::from(&[10u32, 20, 30][..]);
    print_usize(rc_const.len());
    print_u32(rc_const[0] + rc_const[1] + rc_const[2]);

    // `Rc::from(Vec<T>)` (already-written heap buffer -> `allocate_for_ptr_in`)
    let v = vec![4u32, 5, 6, 7];
    let rc_vec: Rc<[u32]> = Rc::from(v.as_slice());
    print_usize(rc_vec.len());
    print_u32(rc_vec[0] + rc_vec[3]);

    // `Arc::from(&[T])` and empty slice
    let arc_slice: Arc<[u16]> = Arc::from(&[7u16, 8, 9][..]);
    print_usize(arc_slice.len());
    print_u32(u32::from(arc_slice[1]));

    let rc_empty: Rc<[u8]> = Rc::from(&[][..]);
    print_usize(rc_empty.len());
}
