//! One-element slices over immutable statics of every element kind: identity and reads.
#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;
use core::ptr;
use core::slice::from_ref;

#[derive(PartialEq)]
struct N { id: u32, w: u64 }
struct Unit;
#[derive(PartialEq)]
enum E { A(u32), B, C { x: u8 } }

// slices declared BEFORE the statics they name, and after
static ONE_EARLY: &[N] = from_ref(&B);
static B: N = N { id: 2, w: 1 << 40 };
static ONE: &[N] = from_ref(&B);
static ONE2: &[N] = from_ref(&B);
static TUP: (u32, u8) = (5, 6);
static ONE_TUP: &[(u32, u8)] = from_ref(&TUP);
static ARR: [u32; 3] = [1, 2, 3];
static ONE_ARR: &[[u32; 3]] = from_ref(&ARR);
static ALL_ARR: &[u32] = &ARR;
static EN: E = E::C { x: 9 };
static ONE_EN: &[E] = from_ref(&EN);
static OPT: Option<u64> = Some(77);
static ONE_OPT: &[Option<u64>] = from_ref(&OPT);
static OPTN: Option<&N> = Some(&B);
static ONE_OPTN: &[Option<&N>] = from_ref(&OPTN);
static Z: () = ();
static ONE_Z: &[()] = from_ref(&Z);
static U: Unit = Unit;
static ONE_U: &[Unit] = from_ref(&U);
static P32: u32 = 7;
static ONE_P32: &[u32] = from_ref(&P32);
static P64: u64 = 1 << 50;
static ONE_P64: &[u64] = from_ref(&P64);
static PB: bool = true;
static ONE_PB: &[bool] = from_ref(&PB);
static PC: char = 'x';
static ONE_PC: &[char] = from_ref(&PC);
static PF: f64 = 2.5;
static ONE_PF: &[f64] = from_ref(&PF);
fn named(x: u32) -> u32 { x + 1 }
static PFN: fn(u32) -> u32 = named;
static ONE_PFN: &[fn(u32) -> u32] = from_ref(&PFN);
static PREF: &u32 = &P32;
static ONE_PREF: &[&u32] = from_ref(&PREF);
static PSTR: &str = "hello";
static ONE_PSTR: &[&str] = from_ref(&PSTR);
// in other positions
static OPT_ONE: Option<&[N]> = Some(from_ref(&B));
struct Holder { name: &'static str, items: &'static [N], count: u32 }
static HOLD: Holder = Holder { name: "h", items: from_ref(&B), count: 1 };
static TABLE: [&[N]; 2] = [from_ref(&B), from_ref(&B2)];
static B2: N = N { id: 3, w: 4 };
struct Raw(*const [N]);
unsafe impl Sync for Raw {}
static RAW: Raw = Raw(from_ref(&B) as *const [N]);
struct RawThin(*const N);
unsafe impl Sync for RawThin {}
static RAWTHIN: RawThin = RawThin(&B as *const N);
// a slice of a slice-holding static
static ONE_HOLD: &[Holder] = from_ref(&HOLD);
// a one-element slice whose element is itself a one-element slice static
static ONE_ONE: &[&[N]] = from_ref(&ONE);

#[inline(never)]
fn sum(s: &[N]) -> u64 { s.iter().map(|n| n.id as u64 + n.w).sum() }

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // aggregate struct
    print_usize(ONE.len()); print_u32(ONE[0].id); print_u64(ONE[black_box(0)].w);
    print_bool(ptr::eq(&ONE[0], &B));
    print_bool(ptr::eq(ONE.as_ptr(), &B));
    print_bool(ptr::eq(ONE_EARLY.as_ptr(), &B));
    print_bool(ptr::eq(ONE.as_ptr(), ONE2.as_ptr()));
    print_bool(ptr::eq(ONE, ONE2));
    print_bool(ptr::eq(ONE, ONE_EARLY));
    print_bool(ptr::eq(ONE, from_ref(black_box(&B))));
    print_bool(ptr::eq(ONE.as_ptr(), &raw const B));
    print_bool(ptr::eq(ONE.first().unwrap(), &B));
    print_bool(ONE.get(1).is_none());
    print_bool(ptr::eq(ONE.iter().next().unwrap(), &B));
    print_bool(unsafe { ONE.as_ptr().add(1) } == ONE.as_ptr_range().end);
    print_usize(unsafe { ONE.as_ptr_range().end.offset_from(ONE.as_ptr()) } as usize);
    print_u64(sum(ONE)); print_u64(sum(black_box(ONE_EARLY)));
    print_bool(ONE.contains(&N { id: 2, w: 1 << 40 }));
    print_bool(ONE == ONE2);
    let (h, t) = ONE.split_first().unwrap();
    print_bool(ptr::eq(h, &B)); print_usize(t.len());
    print_bool(ptr::eq(&ONE[..1][0], &B)); print_usize(ONE[1..].len());
    for n in ONE { print_bool(ptr::eq(n, &B)); }
    // other aggregates
    print_u32(ONE_TUP[0].0 + ONE_TUP[0].1 as u32); print_bool(ptr::eq(ONE_TUP.as_ptr(), &TUP));
    print_u32(ONE_ARR[0][2]); print_bool(ptr::eq(ONE_ARR.as_ptr(), &ARR)); print_usize(ONE_ARR.len());
    print_bool(ptr::eq(ONE_ARR[0].as_ptr(), ALL_ARR.as_ptr())); print_usize(ALL_ARR.len());
    print_bool(ONE_EN[0] == E::C { x: 9 }); print_bool(ptr::eq(ONE_EN.as_ptr(), &EN));
    print_u64(ONE_OPT[0].unwrap_or(0)); print_bool(ptr::eq(ONE_OPT.as_ptr(), &OPT));
    print_u32(ONE_OPTN[0].map_or(0, |n| n.id)); print_bool(ptr::eq(ONE_OPTN.as_ptr(), &OPTN));
    print_bool(ONE_OPTN[0].map_or(false, |n| ptr::eq(n, &B)));
    // zero-sized
    print_usize(ONE_Z.len()); print_usize(ONE_U.len()); print_usize(ONE_Z.iter().count() + ONE_U.iter().count());
    // primitives
    print_u32(ONE_P32[0]); print_bool(ptr::eq(ONE_P32.as_ptr(), &P32)); print_bool(ptr::eq(&ONE_P32[0], &P32));
    print_u64(ONE_P64[0]); print_bool(ptr::eq(ONE_P64.as_ptr(), &P64));
    print_bool(ONE_PB[0]); print_bool(ptr::eq(ONE_PB.as_ptr(), &PB));
    print_u32(ONE_PC[0] as u32); print_bool(ptr::eq(ONE_PC.as_ptr(), &PC));
    print_f64(ONE_PF[0]); print_bool(ptr::eq(ONE_PF.as_ptr(), &PF));
    print_u32(ONE_PFN[0](1)); print_bool(ptr::eq(ONE_PFN.as_ptr(), &PFN));
    print_u32(*ONE_PREF[0]); print_bool(ptr::eq(ONE_PREF.as_ptr(), &PREF)); print_bool(ptr::eq(ONE_PREF[0], &P32));
    print_str(ONE_PSTR[0]); print_bool(ptr::eq(ONE_PSTR.as_ptr(), &PSTR));
    // other positions
    print_bool(OPT_ONE.map_or(false, |s| ptr::eq(s.as_ptr(), &B))); print_usize(OPT_ONE.map_or(9, |s| s.len()));
    print_str(HOLD.name); print_bool(ptr::eq(HOLD.items.as_ptr(), &B)); print_u32(HOLD.items[0].id + HOLD.count);
    print_bool(ptr::eq(TABLE[black_box(0)].as_ptr(), &B)); print_bool(ptr::eq(TABLE[black_box(1)].as_ptr(), &B2));
    print_u64(sum(TABLE[1]));
    print_bool(ptr::eq(RAW.0 as *const N, &B)); print_usize(RAW.0.len());
    print_bool(ptr::eq(RAW.0 as *const N, RAWTHIN.0));
    print_bool(ptr::eq(ONE.as_ptr(), RAWTHIN.0));
    print_u32(unsafe { (*RAW.0)[0].id }); print_u32(unsafe { (*RAWTHIN.0).id });
    print_bool(ptr::eq(ONE_HOLD.as_ptr(), &HOLD)); print_bool(ptr::eq(ONE_HOLD[0].items.as_ptr(), &B));
    print_bool(ptr::eq(ONE_ONE.as_ptr(), &ONE)); print_bool(ptr::eq(ONE_ONE[0].as_ptr(), &B)); print_u32(ONE_ONE[0][0].id);
}
