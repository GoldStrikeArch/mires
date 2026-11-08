//! Nested native `Vec` and `String` values behind one opaque exported Rust aggregate.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

#[path = "prelude.rs"]
mod prelude;

pub struct Row {
    id: u32,
    label: String,
}

pub struct Store {
    rows: Vec<Row>,
}

#[no_mangle]
pub fn native_structural_make() -> Store {
    let mut store = Store {
        rows: Vec::with_capacity(4),
    };
    native_structural_push(&mut store, 7, "alpha");
    native_structural_push(&mut store, 9, "\u{1f980} rust");
    store
}

#[no_mangle]
pub fn native_structural_push(store: &mut Store, id: u32, label: &str) {
    let mut owned = String::new();
    owned.push_str(label);
    store.rows.push(Row { id, label: owned });
}

#[no_mangle]
pub fn native_structural_len(store: &Store) -> usize {
    store.rows.len()
}

#[no_mangle]
pub fn native_structural_id(store: &Store, index: usize) -> u32 {
    store.rows[index].id
}

#[no_mangle]
pub fn native_structural_label(store: &Store, index: usize) -> &str {
    store.rows[index].label.as_str()
}

#[no_mangle]
pub fn native_structural_clear(store: &mut Store) {
    store.rows.clear();
}

#[no_mangle]
pub fn native_structural_round_trip(store: Store) -> Store {
    store
}

#[no_mangle]
pub fn native_structural_dispose(store: Store) -> usize {
    let len = store.rows.len();
    core::mem::drop(store);
    len
}

pub struct UserDrop {
    value: u32,
}

impl Drop for UserDrop {
    fn drop(&mut self) {
        self.value = self.value.wrapping_add(1);
    }
}

#[no_mangle]
pub fn native_structural_own_drop_make(value: u32) -> UserDrop {
    UserDrop { value }
}

#[no_mangle]
pub fn native_structural_own_drop_value(value: &UserDrop) -> u32 {
    value.value
}

#[no_mangle]
pub fn native_structural_own_drop_consume(value: UserDrop) -> u32 {
    value.value
}

#[no_mangle]
pub fn rust_entry() {}
