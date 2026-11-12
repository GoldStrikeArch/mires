//! One-element slices over `static mut` and interior-mutable statics: whole-element stores in both
//! directions.
#![no_std]
#![no_main]
#![allow(static_mut_refs)]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::cell::Cell;
use core::hint::black_box;
use core::ptr;
use core::slice::{from_mut, from_ref};
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

#[derive(Clone, Copy, PartialEq)]
struct Pt { x: u32, y: u32 }

// interior mutability behind a shared one-element slice
struct SyncCell<T>(Cell<T>);
unsafe impl<T> Sync for SyncCell<T> {}
static CP: SyncCell<Pt> = SyncCell(Cell::new(Pt { x: 1, y: 2 }));
static ONE_CP: &[SyncCell<Pt>] = from_ref(&CP);
static CU: SyncCell<u32> = SyncCell(Cell::new(5));
static ONE_CU: &[SyncCell<u32>] = from_ref(&CU);
static CO: SyncCell<Option<u64>> = SyncCell(Cell::new(None));
static ONE_CO: &[SyncCell<Option<u64>>] = from_ref(&CO);
static AT: AtomicU32 = AtomicU32::new(10);
static ONE_AT: &[AtomicU32] = from_ref(&AT);
static AT64: AtomicU64 = AtomicU64::new(1 << 40);
static ONE_AT64: &[AtomicU64] = from_ref(&AT64);
struct Stats { hits: AtomicU32, last: SyncCell<Pt> }
static ST: Stats = Stats { hits: AtomicU32::new(0), last: SyncCell(Cell::new(Pt { x: 0, y: 0 })) };
static ONE_ST: &[Stats] = from_ref(&ST);

// static mut, slices made with from_mut
static mut M: Pt = Pt { x: 1, y: 2 };
static mut M_SL: &mut [Pt] = unsafe { from_mut(&mut *(&raw mut M)) };
static mut MP: u32 = 3;
static mut MP_SL: &mut [u32] = unsafe { from_mut(&mut *(&raw mut MP)) };
static mut M64: u64 = 4;
static mut M64_SL: &mut [u64] = unsafe { from_mut(&mut *(&raw mut M64)) };
static mut MT: (u32, bool) = (5, false);
static mut MT_SL: &mut [(u32, bool)] = unsafe { from_mut(&mut *(&raw mut MT)) };
static mut MA: [u32; 2] = [6, 7];
static mut MA_SL: &mut [[u32; 2]] = unsafe { from_mut(&mut *(&raw mut MA)) };
static mut MO: Option<Pt> = None;
static mut MO_SL: &mut [Option<Pt>] = unsafe { from_mut(&mut *(&raw mut MO)) };
// shared one-element slice of a static mut (read only through it)
static mut MS: Pt = Pt { x: 8, y: 9 };
static MS_SL: &[Pt] = unsafe { from_ref(&*(&raw const MS)) };
// Option / raw forms
static mut MR: Pt = Pt { x: 10, y: 11 };
struct Raw(*mut [Pt]);
unsafe impl Sync for Raw {}
static MR_RAW: Raw = Raw(unsafe { from_mut(&mut *(&raw mut MR)) } as *mut [Pt]);
static mut MQ: Pt = Pt { x: 12, y: 13 };
static mut MQ_OPT: Option<&mut [Pt]> = Some(unsafe { from_mut(&mut *(&raw mut MQ)) });

#[inline(never)]
fn store_all(s: &mut [Pt], v: Pt) { for p in s.iter_mut() { *p = v; } }

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // Cell<Pt>: whole-element stores through the slice element, seen in the static, and back
    ONE_CP[0].0.set(Pt { x: 7, y: 8 });
    print_u32(CP.0.get().x + CP.0.get().y);
    CP.0.set(Pt { x: 20, y: 30 });
    print_u32(ONE_CP[black_box(0)].0.get().x);
    let old = ONE_CP[0].0.replace(Pt { x: 40, y: 50 });
    print_u32(old.y); print_u32(CP.0.get().y);
    let other = Cell::new(Pt { x: 60, y: 70 });
    ONE_CP[0].0.swap(&other);
    print_u32(CP.0.get().x); print_u32(other.get().x);
    print_bool(ptr::eq(ONE_CP.as_ptr(), &CP));
    ONE_CU[0].0.set(55); print_u32(CU.0.get()); CU.0.set(66); print_u32(ONE_CU[0].0.get());
    print_bool(ptr::eq(ONE_CU.as_ptr(), &CU));
    ONE_CO[0].0.set(Some(1 << 33)); print_u64(CO.0.get().unwrap_or(0)); CO.0.set(None); print_bool(ONE_CO[0].0.get().is_none());
    ONE_AT[0].store(11, Ordering::Relaxed); print_u32(AT.load(Ordering::Relaxed));
    AT.fetch_add(5, Ordering::Relaxed); print_u32(ONE_AT[black_box(0)].load(Ordering::Relaxed));
    print_bool(ptr::eq(ONE_AT.as_ptr(), &AT));
    ONE_AT64[0].store(u64::MAX - 1, Ordering::Relaxed); print_u64(AT64.load(Ordering::Relaxed));
    AT64.store(3, Ordering::Relaxed); print_u64(ONE_AT64[0].swap(4, Ordering::Relaxed)); print_u64(AT64.load(Ordering::Relaxed));
    for s in ONE_ST { s.hits.fetch_add(1, Ordering::Relaxed); s.last.0.set(Pt { x: 3, y: 4 }); }
    print_u32(ST.hits.load(Ordering::Relaxed)); print_u32(ST.last.0.get().x + ST.last.0.get().y);
    ST.hits.store(9, Ordering::Relaxed); print_u32(ONE_ST[0].hits.load(Ordering::Relaxed));
    print_bool(ptr::eq(ONE_ST.as_ptr(), &ST));

    unsafe {
        // aggregate static mut
        (*(&raw mut M_SL))[0] = Pt { x: 7, y: 8 }; print_u32(M.x + M.y);
        M = Pt { x: 20, y: 30 }; print_u32((*(&raw const M_SL))[0].x);
        (*(&raw mut M_SL))[0].y = 31; print_u32(M.y);
        M.x = 21; print_u32((*(&raw const M_SL))[0].x);
        let old = core::mem::replace(&mut (*(&raw mut M_SL))[0], Pt { x: 100, y: 200 }); print_u32(old.x + old.y); print_u32(M.x + M.y);
        let mut local = Pt { x: 1000, y: 2000 };
        core::mem::swap(&mut (*(&raw mut M_SL))[0], &mut local); print_u32(M.x + M.y); print_u32(local.x + local.y);
        (*(&raw mut M_SL)).as_mut_ptr().write(Pt { x: 5, y: 6 }); print_u32(M.x + M.y);
        (&raw mut M).write(Pt { x: 50, y: 60 }); print_u32((*(&raw const M_SL))[0].x + (*(&raw const M_SL))[0].y);
        store_all(&mut *(&raw mut M_SL), Pt { x: 9, y: 9 }); print_u32(M.x + M.y);
        print_bool(ptr::eq((*(&raw const M_SL)).as_ptr(), &raw const M));
        print_bool(M == (*(&raw const M_SL))[0]);
        // primitive static mut
        (*(&raw mut MP_SL))[0] = 33; print_u32(MP); MP = 44; print_u32((*(&raw const MP_SL))[0]);
        (*(&raw mut MP_SL))[black_box(0)] += 1; print_u32(MP);
        print_bool(ptr::eq((*(&raw const MP_SL)).as_ptr(), &raw const MP));
        (*(&raw mut M64_SL))[0] = 1 << 60; print_u64(M64); M64 = 5; print_u64((*(&raw const M64_SL))[0]);
        // tuple, array, option elements
        (*(&raw mut MT_SL))[0] = (50, true); print_u32(MT.0); print_bool(MT.1);
        MT = (51, false); print_u32((*(&raw const MT_SL))[0].0);
        (*(&raw mut MA_SL))[0] = [60, 70]; print_u32(MA[0] + MA[1]);
        MA = [61, 71]; print_u32((*(&raw const MA_SL))[0][1]);
        (*(&raw mut MA_SL))[0][1] = 72; print_u32(MA[1]);
        (*(&raw mut MO_SL))[0] = Some(Pt { x: 1, y: 1 }); print_bool(MO.is_some());
        MO = None; print_bool((*(&raw const MO_SL))[0].is_none());
        MO = Some(Pt { x: 2, y: 3 }); print_u32((*(&raw const MO_SL))[0].map_or(0, |p| p.x + p.y));
        // shared slice of a static mut: later stores to the static are seen
        print_u32(MS_SL[0].x); MS = Pt { x: 80, y: 90 }; print_u32(MS_SL[0].x + MS_SL[black_box(0)].y);
        MS.y = 91; print_u32(MS_SL[0].y); print_bool(ptr::eq(MS_SL.as_ptr(), &raw const MS));
        // raw and Option forms
        (*MR_RAW.0)[0] = Pt { x: 7, y: 8 }; print_u32(MR.x + MR.y);
        MR = Pt { x: 1, y: 1 }; print_u32((*MR_RAW.0)[0].x + (*MR_RAW.0)[0].y);
        print_bool(ptr::eq(MR_RAW.0 as *mut Pt, &raw mut MR));
        if let Some(s) = &mut *(&raw mut MQ_OPT) { s[0] = Pt { x: 70, y: 80 }; }
        print_u32(MQ.x + MQ.y);
        MQ = Pt { x: 2, y: 2 };
        if let Some(s) = &*(&raw const MQ_OPT) { print_u32(s[0].x + s[0].y); print_bool(ptr::eq(s.as_ptr(), &raw const MQ)); }
    }
}
