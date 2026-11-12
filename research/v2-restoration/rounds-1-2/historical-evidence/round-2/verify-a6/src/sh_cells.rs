#![no_std]
#![no_main]
#![allow(dead_code, static_mut_refs, unused)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::cell::Cell;
use core::sync::atomic::{AtomicU32, AtomicU64, AtomicBool, Ordering};
struct A { hits: AtomicU32, big: AtomicU64, flag: AtomicBool, peer: &'static A }
static A1: A = A { hits: AtomicU32::new(1), big: AtomicU64::new(1 << 40), flag: AtomicBool::new(false), peer: &A2 };
static A2: A = A { hits: AtomicU32::new(10), big: AtomicU64::new(5), flag: AtomicBool::new(true), peer: &A1 };
static AS: A = A { hits: AtomicU32::new(100), big: AtomicU64::new(0), flag: AtomicBool::new(false), peer: &AS };
static CNT: AtomicU32 = AtomicU32::new(3);
struct U { c: &'static AtomicU32, me: &'static U, arr: [AtomicU32; 3] }
static U1: U = U { c: &CNT, me: &U1, arr: [AtomicU32::new(1), AtomicU32::new(2), AtomicU32::new(3)] };
struct C { v: Cell<u32>, arr: [Cell<u8>; 2], peer: *const C }
static mut C1: C = C { v: Cell::new(1), arr: [Cell::new(1), Cell::new(2)], peer: unsafe { core::ptr::addr_of!(C2) } };
static mut C2: C = C { v: Cell::new(2), arr: [Cell::new(3), Cell::new(4)], peer: unsafe { core::ptr::addr_of!(C1) } };
struct MN { id: u32, count: u32, next: *mut MN }
static mut MN1: MN = MN { id: 1, count: 0, next: unsafe { core::ptr::addr_of_mut!(MN2) } };
static mut MN2: MN = MN { id: 2, count: 0, next: unsafe { core::ptr::addr_of_mut!(MN1) } };
static mut MSELF: MN = MN { id: 3, count: 0, next: unsafe { core::ptr::addr_of_mut!(MSELF) } };
#[unsafe(no_mangle)]
pub fn rust_entry() {
    A1.peer.hits.fetch_add(5, Ordering::Relaxed);
    A2.peer.peer.hits.fetch_add(7, Ordering::Relaxed);
    print_u32(A2.hits.load(Ordering::Relaxed));
    print_u32(A1.peer.hits.load(Ordering::Relaxed));
    A1.big.fetch_add(1, Ordering::Relaxed);
    print_u64(A2.peer.big.load(Ordering::Relaxed));
    A2.peer.flag.store(true, Ordering::Relaxed);
    print_bool(A1.flag.load(Ordering::Relaxed) && A1.peer.flag.load(Ordering::Relaxed));
    AS.peer.peer.hits.store(4, Ordering::Relaxed);
    print_u32(AS.hits.load(Ordering::Relaxed));
    U1.me.c.fetch_add(1, Ordering::Relaxed);
    U1.me.me.arr[1].fetch_add(40, Ordering::Relaxed);
    print_u32(CNT.load(Ordering::Relaxed) + U1.arr[1].load(Ordering::Relaxed));
    print_bool(core::ptr::eq(U1.c, &CNT));
    unsafe {
        let c1 = &*core::ptr::addr_of!(C1);
        let c2 = &*c1.peer;
        c2.v.set(c2.v.get() + 40);
        (*c2.peer).v.set(7);
        (*c2.peer).arr[1].set(9);
        print_u32((*core::ptr::addr_of!(C2)).v.get());
        print_u32(c1.v.get() + c1.arr[1].get() as u32);
        print_bool(core::ptr::eq(c2.peer, core::ptr::addr_of!(C1)));
        print_bool(core::ptr::eq((*c2.peer).peer, core::ptr::addr_of!(C2)));
        let m1 = core::ptr::addr_of_mut!(MN1);
        (*(*m1).next).count += 3;
        (*(*(*m1).next).next).count += 5;
        MN2.count += 100;
        print_u32(MN1.count * 1000 + MN2.count);
        print_u32((*MN1.next).id * 10 + (*(*MN1.next).next).id);
        print_bool((*MN1.next).next == m1);
        (*MSELF.next).count = 8;
        (*(*MSELF.next).next).count += 1;
        print_u32(MSELF.count);
        print_bool(MSELF.next == core::ptr::addr_of_mut!(MSELF));
        MN1.next = core::ptr::addr_of_mut!(MSELF);
        print_u32((*(*MN1.next).next).id);
    }
}
