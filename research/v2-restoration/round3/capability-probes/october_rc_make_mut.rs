#![no_std]
#![no_main]
extern crate alloc;
#[path = "../../../../examples/core-tests/prelude.rs"] mod prelude;
use prelude::*;
#[no_mangle] pub fn rust_entry() { let mut a = alloc::rc::Rc::new(core::hint::black_box(7u32)); let b = a.clone(); *alloc::rc::Rc::make_mut(&mut a) = 9; print_u32(*a); print_u32(*b); print_u32(alloc::rc::Rc::strong_count(&a) as u32); drop(b); *alloc::rc::Rc::make_mut(&mut a) += 1; print_u32(*a); }
