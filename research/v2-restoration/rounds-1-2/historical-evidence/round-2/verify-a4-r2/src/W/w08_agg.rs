#![no_std]
#![no_main]
#![allow(unused, static_mut_refs, dangerous_implicit_autorefs)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::hint::black_box;
use core::sync::atomic::{AtomicU32, AtomicU8, Ordering};
#[inline(never)] fn sum(s: &[u32]) -> u32 { let mut t = 0; for v in s { t = t * 3 + *v; } t }
#[inline(never)] fn show(s: &[u32]) { print_usize(s.len()); print_u32(sum(s)); }
#[inline(never)] fn showo(o: Option<&[u32]>) { match o { Some(s) => show(s), None => print_str("none") } }

#[derive(Clone, Copy)] struct Pt { x: u32, y: u32 }
static PTS: [Pt; 3] = [Pt { x: 1, y: 2 }, Pt { x: 3, y: 4 }, Pt { x: 5, y: 6 }];
static PS: &[Pt] = &PTS;
static PT: &[Pt] = PTS.split_at(1).1;
static PO: Option<&[Pt]> = Some(PTS.split_at(2).1);
static TUP: [(u8, u64); 2] = [(1, 10_000_000_000), (2, 3)];
static TS: &[(u8, u64)] = &TUP;
static OPTS: [Option<u32>; 3] = [Some(1), None, Some(3)];
static OS: &[Option<u32>] = OPTS.split_at(1).1;
static STRS: [&str; 3] = ["a", "bc", "def"];
static SS: &[&str] = STRS.split_at(1).1;
static F: [f64; 3] = [1.5, 2.5, 3.5];
static FS: &[f64] = F.split_at(1).1;
static B: [bool; 3] = [true, false, true];
static BS: &[bool] = B.split_at(2).1;
static C: [char; 3] = ['a', 'b', 'c'];
static CS: &[char] = C.split_at(1).1;
static L: [u64; 3] = [1, 0xFFFF_FFFF_FFFF, 3];
static LS: &[u64] = L.split_at(1).1;
static I: [i64; 2] = [-5, 6];
static IS: &[i64] = &I;
static H: [u16; 4] = [1, 2, 3, 65535];
static HS: &[u16] = H.split_at(3).1;
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_usize(PS.len()); print_u32(PS[2].x * 10 + PS[2].y); print_usize(PT.len()); print_u32(PT[0].x * 10 + PT[0].y);
    print_u32(PO.map_or(0, |s| s[0].y)); print_u32(black_box(PO).map_or(0, |s| s.len() as u32));
    print_bool(core::ptr::eq(&PT[0], &PTS[1])); print_bool(core::ptr::eq(PT.as_ptr(), &PTS[1])); print_bool(core::ptr::eq(PS, &PTS[..]));
    print_usize(TS.len()); print_u64(TS[0].1); print_u32(TS[1].0 as u32);
    print_usize(OS.len()); print_u32(OS[0].unwrap_or(77)); print_u32(OS[1].unwrap_or(77));
    print_usize(SS.len()); print_str(SS[0]); print_str(SS[1]);
    print_usize(FS.len()); print_f64(FS[0] + FS[1]);
    print_usize(BS.len()); print_bool(BS[0]);
    print_usize(CS.len()); print_u32(CS[1] as u32);
    print_usize(LS.len()); print_u64(LS[0]); print_u64(LS[1]);
    print_usize(IS.len()); print_i64(IS[0]);
    print_usize(HS.len()); print_u32(HS[0] as u32);
    print_bool(core::ptr::eq(LS.as_ptr(), &L[1])); print_bool(core::ptr::eq(HS.as_ptr(), &H[3])); print_bool(core::ptr::eq(FS.as_ptr(), &F[1]));
}
