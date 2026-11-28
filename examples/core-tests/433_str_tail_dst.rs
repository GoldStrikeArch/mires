//! Regression test for structs with an unsized `str` tail (like `regex-lite`'s DST pattern struct):
//! heap allocation via `ptr::from_raw_parts_mut`, header + UTF-8 tail writes, `&str` reads
//! (ASCII and multi-byte UTF-8), and `&mut str` in-place mutation.

#![no_std]
#![no_main]
#![feature(ptr_metadata)]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::alloc::{alloc, dealloc, Layout};
use alloc::rc::Rc;
use alloc::sync::Arc;
use core::ptr;

#[repr(C)]
struct TaggedStr {
    tag: u32,
    text: str,
}

fn alloc_tagged_str(tag: u32, s: &str) -> (*mut TaggedStr, Layout) {
    let header_layout = Layout::new::<u32>();
    let tail_layout = Layout::array::<u8>(s.len()).unwrap();
    let (layout, _) = header_layout.extend(tail_layout).unwrap();
    let layout = layout.pad_to_align();
    unsafe {
        let raw = alloc(layout);
        let dst: *mut TaggedStr = ptr::from_raw_parts_mut(raw, s.len());
        ptr::addr_of_mut!((*dst).tag).write(tag);
        let tail_ptr = ptr::addr_of_mut!((*dst).text) as *mut u8;
        ptr::copy_nonoverlapping(s.as_ptr(), tail_ptr, s.len());
        (dst, layout)
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let (p1, l1) = alloc_tagged_str(7, "hello");
    unsafe {
        print_u32((*p1).tag);
        print_str(&(*p1).text);
        print_usize((&(*p1).text).len());
        (&mut (*p1).text).make_ascii_uppercase();
        print_str(&(*p1).text);
        dealloc(p1 as *mut u8, l1);
    }

    let (p2, l2) = alloc_tagged_str(99, "café");
    unsafe {
        print_u32((*p2).tag);
        print_str(&(*p2).text);
        print_usize((&(*p2).text).len());
        dealloc(p2 as *mut u8, l2);
    }

    let r1: Rc<str> = Rc::from("statue");
    let r2 = Rc::clone(&r1);
    print_str(&r1);
    print_usize(r2.len());
    drop(r1);
    drop(r2);

    let a1: Arc<str> = Arc::from("eggplant café");
    let a2 = Arc::clone(&a1);
    print_str(&a1);
    print_usize(a2.len());
    drop(a1);
    drop(a2);
}
