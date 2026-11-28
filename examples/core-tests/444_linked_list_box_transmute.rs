//! `alloc::collections::LinkedList` exercising structural aggregate transmute of
//! `Box<MaybeUninit<Node<T>>, &Global> -> Box<Node<T>, &Global>`.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::collections::LinkedList;
use alloc::string::String;
use core::hint::black_box;

#[unsafe(no_mangle)]
pub extern "C" fn rust_entry() {
    let mut list: LinkedList<i32> = LinkedList::new();
    list.push_back(black_box(10));
    list.push_back(black_box(20));
    list.push_front(black_box(5));
    list.push_back(black_box(30));

    print_usize(list.len());
    let mut sum = 0i32;
    for &x in &list {
        sum += x;
    }
    print_i32(sum);

    for x in &mut list {
        *x += 1;
    }
    print_i32(list.pop_front().unwrap());
    print_i32(list.pop_back().unwrap());

    let mut tail = list.split_off(1);
    print_usize(list.len());
    print_usize(tail.len());
    list.append(&mut tail);
    print_usize(list.len());
    print_i32(list.pop_front().unwrap());
    print_i32(list.pop_front().unwrap());
    print_bool(list.is_empty());

    // Owned String nodes (tests Drop + Box::new_in with &Global allocator)
    let mut words: LinkedList<String> = LinkedList::new();
    words.push_back(String::from("first"));
    words.push_back(String::from("second"));
    words.push_front(String::from("zero"));
    print_str(&words.pop_front().unwrap());
    print_str(&words.pop_back().unwrap());
    print_str(words.front().unwrap());
    drop(words);
    print_bool(true);
}
