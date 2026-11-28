//! Regression test for `CastKind::DynUpcast { slot }` (`JIR_SCHEMA = 25`):
//! primary supertraits, non-primary supertraits, multi-level diamond upcasts, auto-trait upcasts,
//! `Box<dyn Sub> -> Box<dyn Super>`, `Pin<Box<dyn Sub>> -> Pin<Box<dyn Super>>`,
//! `Holder<Concrete> -> Holder<dyn Sub> -> Holder<dyn Super>` (`dyn`-tail structs for both
//! indirect structs and direct primitives), and multi-field `CoerceUnsized` smart pointers.

#![no_std]
#![no_main]
#![feature(coerce_unsized, unsize)]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::boxed::Box;
use core::marker::Unsize;
use core::ops::CoerceUnsized;
use core::pin::Pin;

trait Base {
    fn base_id(&self) -> i32;
}

trait SuperA: Base {
    fn a_val(&self) -> i32;
}

trait SuperB: Base {
    fn b_val(&self) -> i32;
}

trait Sub: SuperA + SuperB {
    fn sub_val(&self) -> i32;
}

struct Widget {
    a: i32,
    b: i32,
}

struct Holder<T: ?Sized> {
    tag: i32,
    inner: T,
}

struct SmartRef<'a, T: ?Sized> {
    label: i32,
    ptr: &'a T,
}

impl<'a, T: ?Sized + Unsize<U>, U: ?Sized> CoerceUnsized<SmartRef<'a, U>> for SmartRef<'a, T> {}

impl Base for Widget {
    fn base_id(&self) -> i32 {
        self.a * 10 + self.b
    }
}
impl SuperA for Widget {
    fn a_val(&self) -> i32 {
        self.a
    }
}
impl SuperB for Widget {
    fn b_val(&self) -> i32 {
        self.b
    }
}
impl Sub for Widget {
    fn sub_val(&self) -> i32 {
        self.a + self.b
    }
}

impl Base for i32 {
    fn base_id(&self) -> i32 {
        *self
    }
}
impl SuperA for i32 {
    fn a_val(&self) -> i32 {
        *self + 1
    }
}
impl SuperB for i32 {
    fn b_val(&self) -> i32 {
        *self + 2
    }
}
impl Sub for i32 {
    fn sub_val(&self) -> i32 {
        *self + 3
    }
}

#[inline(never)]
fn upcast_ref_a(s: &dyn Sub) -> &dyn SuperA {
    s
}

#[inline(never)]
fn upcast_ref_b(s: &dyn Sub) -> &dyn SuperB {
    s
}

#[inline(never)]
fn upcast_b_to_base(s: &dyn SuperB) -> &dyn Base {
    s
}

#[inline(never)]
fn upcast_box_b(s: Box<dyn Sub>) -> Box<dyn SuperB> {
    s
}

#[inline(never)]
fn upcast_holder_b(h: &Holder<dyn Sub>) -> &Holder<dyn SuperB> {
    h
}

#[inline(never)]
fn upcast_pin_box_b(s: Pin<Box<dyn Sub>>) -> Pin<Box<dyn SuperB>> {
    s
}

#[inline(never)]
fn upcast_smart_ref_b<'a>(s: SmartRef<'a, dyn Sub>) -> SmartRef<'a, dyn SuperB> {
    s
}

#[inline(never)]
fn upcast_auto(s: &(dyn Sub + Send + Sync)) -> &(dyn Send + Sync) {
    s
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let w = Widget { a: 3, b: 7 };
    let sub_ref: &dyn Sub = &w;
    print_i32(sub_ref.sub_val());
    print_i32(upcast_ref_a(sub_ref).a_val());
    let b_ref = upcast_ref_b(sub_ref);
    print_i32(b_ref.b_val());
    print_i32(upcast_b_to_base(b_ref).base_id());

    // Auto-trait upcast (`slot == None` with no principal trait)
    let auto_ref = upcast_auto(&w);
    let auto_ptr: *const (dyn Send + Sync) = auto_ref;
    print_bool(!auto_ptr.is_null());

    // Box<dyn Sub> -> Box<dyn SuperB> (transparent wrapper around raw ptr)
    let bx: Box<dyn Sub> = Box::new(Widget { a: 4, b: 9 });
    let bx_b = upcast_box_b(bx);
    print_i32(bx_b.b_val());
    print_i32(bx_b.base_id());

    // Unsized `dyn`-tail struct: concrete -> `dyn Sub` -> `dyn SuperB` (indirect & primitive)
    let h_widget = Holder {
        tag: 100,
        inner: Widget { a: 2, b: 5 },
    };
    let h_sub: &Holder<dyn Sub> = &h_widget;
    print_i32(h_sub.tag);
    print_i32(h_sub.inner.sub_val());
    let h_b = upcast_holder_b(h_sub);
    print_i32(h_b.inner.b_val());
    print_i32(h_b.inner.base_id());

    let h_prim = Holder { tag: 200, inner: 40i32 };
    let hp_sub: &Holder<dyn Sub> = &h_prim;
    print_i32(hp_sub.tag);
    print_i32(hp_sub.inner.sub_val());
    let hp_b = upcast_holder_b(hp_sub);
    print_i32(hp_b.inner.b_val());
    print_i32(hp_b.inner.base_id());

    // Multi-layer transparent wrapper (`Pin<Box<dyn Sub>> -> Pin<Box<dyn SuperB>>`)
    let pbx: Pin<Box<dyn Sub>> = Box::into_pin(Box::new(Widget { a: 6, b: 8 }));
    let pbx_b = upcast_pin_box_b(pbx);
    print_i32(pbx_b.b_val());
    print_i32(pbx_b.base_id());

    // Multi-field non-transparent `CoerceUnsized` struct (`SmartRef<dyn Sub> -> SmartRef<dyn SuperB>`)
    let w2 = Widget { a: 8, b: 3 };
    let sr_sub: SmartRef<'_, dyn Sub> = SmartRef { label: 77, ptr: &w2 };
    let sr_b = upcast_smart_ref_b(sr_sub);
    print_i32(sr_b.label);
    print_i32(sr_b.ptr.b_val());
    print_i32(sr_b.ptr.base_id());
}
