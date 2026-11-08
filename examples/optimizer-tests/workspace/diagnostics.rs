#![feature(no_core, lang_items, intrinsics, rustc_attrs, decl_macro)]
#![feature(auto_traits, freeze_impls, unboxed_closures)]
#![allow(internal_features, dead_code, unused_variables)]
#![no_core]
#![no_main]

#[path = "../../mini_core.rs"]
mod mini_core;
use mini_core::*;

#[path = "../dependency.rs"]
mod dependency;

struct WorkspaceValue<T> {
    value: T,
}

#[inline(never)]
fn rejected_workspace_allocation<T: Copy>(value: T) -> i32 {
    let wrapped = WorkspaceValue { value };
    observe(&wrapped)
}

#[inline(never)]
fn observe<T>(_: &WorkspaceValue<T>) -> i32 {
    1
}

#[no_mangle]
fn rust_entry() {
    print_i32(rejected_workspace_allocation(11_i32));
    print_i32(rejected_workspace_allocation(12_u32));
    print_i32(dependency::rejected_dependency_allocation());
}
