//! Moving an owned `Vec<Row>` into another vector through idiomatic `Extend`.

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

fn row(id: u32, label: &str) -> Row {
    let mut owned = String::new();
    owned.push_str(label);
    Row { id, label: owned }
}

#[no_mangle]
pub fn native_vec_extend_target() -> Vec<Row> {
    let mut rows = Vec::with_capacity(2);
    rows.push(row(1, "one"));
    rows.push(row(2, "two"));
    rows
}

#[no_mangle]
pub fn native_vec_extend_source() -> Vec<Row> {
    let mut rows = Vec::with_capacity(4);
    rows.push(row(3, "three"));
    rows.push(row(4, "four"));
    rows.push(row(5, "five"));
    rows
}

#[no_mangle]
pub fn native_vec_extend(target: &mut Vec<Row>, source: Vec<Row>) {
    target.extend(source);
}

#[no_mangle]
pub fn native_vec_extend_dispose(rows: Vec<Row>) {
    core::mem::drop(rows);
}

#[no_mangle]
pub fn native_vec_extend_len(rows: &Vec<Row>) -> usize {
    rows.len()
}

#[no_mangle]
pub fn native_vec_extend_capacity(rows: &Vec<Row>) -> usize {
    rows.capacity()
}

#[no_mangle]
pub fn native_vec_extend_id(rows: &Vec<Row>, index: usize) -> u32 {
    rows[index].id
}

#[no_mangle]
pub fn native_vec_extend_label(rows: &Vec<Row>, index: usize) -> &str {
    rows[index].label.as_str()
}

#[no_mangle]
pub fn native_vec_extend_iterator_make() -> Vec<u32> {
    let mut values = Vec::new();
    values.push(3);
    values.push(5);
    values
}

#[no_mangle]
pub fn native_vec_extend_iterator(values: &mut Vec<u32>) {
    values.extend([8, 13]);
}

#[no_mangle]
pub fn native_vec_extend_iterator_get(values: &Vec<u32>, index: usize) -> u32 {
    values[index]
}

#[no_mangle]
pub fn rust_entry() {}
