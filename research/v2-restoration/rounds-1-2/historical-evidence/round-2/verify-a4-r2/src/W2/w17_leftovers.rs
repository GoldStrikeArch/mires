#![no_std]
#![no_main]
#![allow(unused)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::hint::black_box;

#[derive(Clone, Copy, PartialEq)] struct Zst;
static U: () = ();
static US: &[()] = core::slice::from_ref(&U);
static Z: Zst = Zst;
static ZS: &[Zst] = core::slice::from_ref(&Z);
static ZO: Option<&[Zst]> = Some(core::slice::from_ref(&Z));
struct Named { name: [u8; 3], n: u32 }
static ST: Named = Named { name: *b"abc", n: 1 };
static T: &str = match core::str::from_utf8(&ST.name) { Ok(s) => s, Err(_) => "bad" };
static B: &[u8; 5] = b"hello";
static S: &str = match core::str::from_utf8(B) { Ok(s) => s, Err(_) => "bad" };
static ONE: u64 = 0x1_0000_0001;
static ONE_SL: &[u64] = core::slice::from_ref(&ONE);
static ONE_END: &[u64] = core::slice::from_ref(&ONE).split_at(1).1;
static FLAG: bool = true;
static FLAGS: &[bool] = core::slice::from_ref(&FLAG);
static CH: char = 'x';
static CHS: Option<&[char]> = Some(core::slice::from_ref(&CH));
static TUP: (u32, u8) = (5, 6);
static TUPS: &[(u32, u8)] = core::slice::from_ref(&TUP);
static OPTV: Option<u32> = Some(9);
static OPTS: &[Option<u32>] = core::slice::from_ref(&OPTV);
static NAME: &str = "nm";
static NAMES: &[&str] = core::slice::from_ref(&NAME);
static INNER: [u32; 2] = [1, 2];
static SLV: &[u32] = &INNER;
static SLVS: &[&[u32]] = core::slice::from_ref(&SLV);

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_usize(US.len()); print_usize(US.iter().count()); print_usize(ZS.len()); print_bool(ZS[0] == Zst);
    print_usize(ZO.map_or(9, |s| s.len())); print_usize(black_box(ZO).map_or(9, |s| s.iter().count()));
    print_str(T); print_usize(T.len()); print_u32(ST.n); print_str(S);
    print_usize(ONE_SL.len()); print_u64(ONE_SL[0]); print_usize(ONE_END.len()); print_bool(core::ptr::eq(&ONE_SL[0], &ONE));
    print_usize(FLAGS.len()); print_bool(FLAGS[0]); print_bool(core::ptr::eq(&FLAGS[0], &FLAG));
    print_u32(CHS.map_or(0, |s| s[0] as u32)); print_usize(black_box(CHS).map_or(0, |s| s.len()));
    print_usize(TUPS.len()); print_u32(TUPS[0].0 + TUPS[0].1 as u32); print_bool(core::ptr::eq(&TUPS[0], &TUP));
    print_usize(OPTS.len()); print_u32(OPTS[0].unwrap_or(0)); print_bool(core::ptr::eq(&OPTS[0], &OPTV));
    print_usize(NAMES.len()); print_str(NAMES[0]);
    print_usize(SLVS.len()); print_usize(SLVS[0].len()); print_u32(SLVS[0][1]); print_bool(core::ptr::eq(SLVS[0], SLV)); print_bool(core::ptr::eq(SLVS[0].as_ptr(), INNER.as_ptr()));
}
