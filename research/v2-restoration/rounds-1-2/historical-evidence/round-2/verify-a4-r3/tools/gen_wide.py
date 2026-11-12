#!/usr/bin/env python3
"""gen_wide.py <outdir>: family W, wide pointers (&[T], &str) into statics. One program each."""
import os, sys

HEAD = '''#![no_std]
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
'''
P = {}
P['w01_ranges'] = '''
static ARR: [u32; 4] = [10, 20, 30, 40];
static WHOLE: &[u32] = &ARR;
static PRE: &[u32] = ARR.split_at(2).0;
static SUF: &[u32] = ARR.split_at(1).1;
static MID: &[u32] = ARR.split_at(1).1.split_at(2).0;
static E0: &[u32] = ARR.split_at(0).0;
static E2: &[u32] = ARR.split_at(2).1.split_at(0).0;
static E4: &[u32] = ARR.split_at(4).1;
static ONE: &[u32] = core::slice::from_ref(&ARR[2]);
static VIA: &[u32; 4] = &ARR;
static VIA_SL: &[u32] = VIA;
static AS_SL: &[u32] = ARR.as_slice();
#[unsafe(no_mangle)]
pub fn rust_entry() {
    show(WHOLE); show(PRE); show(SUF); show(MID); show(E0); show(E2); show(E4); show(ONE);
    show(black_box(WHOLE)); show(black_box(MID)); show(black_box(E4));
    show(VIA_SL); show(AS_SL); print_u32(VIA[3]);
    print_bool(core::ptr::eq(WHOLE.as_ptr(), ARR.as_ptr()));
    print_bool(core::ptr::eq(WHOLE.as_ptr(), &ARR[0]));
    print_bool(core::ptr::eq(SUF.as_ptr(), &ARR[1]));
    print_bool(core::ptr::eq(MID.as_ptr(), SUF.as_ptr()));
    print_bool(core::ptr::eq(MID, &ARR[1..3]));
    print_bool(core::ptr::eq(MID, SUF));
    print_bool(core::ptr::eq(WHOLE, &ARR[..]));
    print_bool(core::ptr::eq(WHOLE, AS_SL));
    print_bool(core::ptr::eq(&WHOLE[2], &ARR[2]));
    print_bool(core::ptr::eq(&WHOLE[2], &MID[1]));
    print_bool(core::ptr::eq(ONE.as_ptr(), &ARR[2]));
    print_bool(core::ptr::eq(E4.as_ptr(), ARR.as_ptr_range().end));
    print_bool(core::ptr::eq(E0.as_ptr(), ARR.as_ptr()));
    print_bool(core::ptr::eq(WHOLE[1..].as_ptr(), SUF.as_ptr()));
    print_bool(core::ptr::eq(VIA, &ARR));
    print_usize(unsafe { SUF.as_ptr().offset_from(ARR.as_ptr()) } as usize);
    print_usize(unsafe { E4.as_ptr().offset_from(WHOLE.as_ptr()) } as usize);
    let rt: &[u32] = &ARR[black_box(1)..];
    print_bool(core::ptr::eq(rt, SUF)); print_bool(rt == SUF); print_bool(WHOLE == &ARR[..]);
    let mut it = SUF.iter(); print_u32(*it.next().unwrap()); print_usize(it.len());
    print_u32(*SUF.last().unwrap()); print_u32(SUF.iter().rev().fold(0, |a, b| a * 2 + *b));
    print_bool(SUF.contains(&40)); print_bool(PRE.contains(&40));
    let (a, b) = WHOLE.split_at(black_box(2)); show(a); show(b);
    print_bool(E0.is_empty()); print_bool(E4.first().is_none()); print_bool(WHOLE.get(4).is_none()); print_u32(*WHOLE.get(3).unwrap());
}
'''
P['w01b_split_first'] = '''
static ARR: [u32; 4] = [10, 20, 30, 40];
static FIRST: Option<(&u32, &[u32])> = ARR.split_first();
#[unsafe(no_mangle)]
pub fn rust_entry() {
    match FIRST { Some((a, r)) => { print_u32(*a); show(r); print_bool(core::ptr::eq(a, &ARR[0])); print_bool(core::ptr::eq(r.as_ptr(), &ARR[1])); } None => print_str("none") }
}
'''
P['w01c_split_last'] = '''
static ARR: [u32; 4] = [10, 20, 30, 40];
static LAST: Option<(&u32, &[u32])> = ARR.split_last();
#[unsafe(no_mangle)]
pub fn rust_entry() {
    match LAST { Some((a, r)) => { print_u32(*a); show(r); } None => print_str("none") }
}
'''
P['w01d_chunk'] = '''
static ARR: [u32; 4] = [10, 20, 30, 40];
static CHUNK: Option<&[u32; 2]> = ARR.first_chunk::<2>();
static LCHUNK: Option<&[u32; 2]> = ARR.last_chunk::<2>();
#[unsafe(no_mangle)]
pub fn rust_entry() {
    match CHUNK { Some(c) => { print_u32(c[0] + c[1]); } None => print_str("none") }
    match LCHUNK { Some(c) => { print_u32(c[0] + c[1]); } None => print_str("none") }
}
'''
P['w02_zst'] = '''
#[derive(Clone, Copy, PartialEq)] struct Z;
static ZA: [(); 4] = [(); 4];
static ZS: &[()] = &ZA;
static ZT: &[()] = ZA.split_at(2).1;
static ZE: &[()] = ZA.split_at(4).1;
static ZZ: [Z; 3] = [Z, Z, Z];
static ZZS: &[Z] = &ZZ;
static ZO: Option<&[Z]> = Some(ZZ.split_at(1).1);
static E: [u32; 0] = [];
static ES: &[u32] = &E;
static EO: Option<&[u32]> = Some(&E);
static ONE: [u32; 1] = [9];
static OS: &[u32] = &ONE;
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_usize(ZS.len()); print_usize(ZT.len()); print_usize(ZE.len()); print_usize(ZZS.len());
    print_usize(black_box(ZS).iter().count()); print_usize(black_box(ZT).iter().count());
    print_usize(ZO.map_or(99, |s| s.len())); print_usize(black_box(ZO).map_or(99, |s| s.iter().filter(|z| **z == Z).count()));
    print_bool(ZS.get(3).is_some()); print_bool(ZS.get(4).is_some()); print_bool(ZT.get(2).is_some());
    show(ES); showo(EO); showo(black_box(EO)); show(OS);
    print_bool(core::ptr::eq(OS.as_ptr(), &ONE[0])); print_bool(core::ptr::eq(ES.as_ptr(), E.as_ptr()));
    print_bool(core::ptr::eq(ZS.as_ptr(), ZA.as_ptr()));
}
'''
P['w03_field'] = '''
struct St { a: u32, arr: [u32; 3], b: u8 }
static ST: St = St { a: 1, arr: [5, 6, 7], b: 2 };
static SL: &[u32] = &ST.arr;
#[unsafe(no_mangle)]
pub fn rust_entry() { show(SL); show(black_box(SL)); print_bool(core::ptr::eq(SL.as_ptr(), ST.arr.as_ptr())); print_u32(ST.a + ST.b as u32); }
'''
P['w03b_field_first'] = '''
struct St { arr: [u32; 3] }
static ST: St = St { arr: [5, 6, 7] };
static SL: &[u32] = &ST.arr;
#[unsafe(no_mangle)]
pub fn rust_entry() { show(SL); show(black_box(SL)); print_bool(core::ptr::eq(SL.as_ptr(), ST.arr.as_ptr())); }
'''
P['w03c_tuple_field'] = '''
static TU: (u8, [u32; 2]) = (1, [8, 9]);
static SL: &[u32] = &TU.1;
#[unsafe(no_mangle)]
pub fn rust_entry() { show(SL); print_u32(TU.0 as u32); }
'''
P['w04_nested'] = '''
static M: [[u32; 2]; 3] = [[1, 2], [3, 4], [5, 6]];
static ROWS: &[[u32; 2]] = &M;
static TAILROWS: &[[u32; 2]] = M.split_at(1).1;
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_usize(ROWS.len()); print_u32(ROWS[2][1]); show(&ROWS[1]); print_usize(TAILROWS.len()); show(&TAILROWS[0]);
    print_bool(core::ptr::eq(&TAILROWS[0], &M[1])); print_bool(core::ptr::eq(ROWS.as_ptr(), M.as_ptr()));
    print_bool(core::ptr::eq(&TAILROWS[1][1], &M[2][1]));
}
'''
P['w04b_row'] = '''
static M: [[u32; 2]; 3] = [[1, 2], [3, 4], [5, 6]];
static ROW: &[u32] = &M[1];
#[unsafe(no_mangle)]
pub fn rust_entry() { show(ROW); print_bool(core::ptr::eq(ROW.as_ptr(), M[1].as_ptr())); }
'''
P['w04c_flat'] = '''
static M: [[u32; 2]; 3] = [[1, 2], [3, 4], [5, 6]];
static FLAT: &[u32] = M.as_flattened();
#[unsafe(no_mangle)]
pub fn rust_entry() { show(FLAT); }
'''
P['w05_static_mut'] = '''
static mut BUF: [u32; 4] = [1, 2, 3, 4];
static mut SLM: &mut [u32] = unsafe { &mut *(&raw mut BUF) };
static mut TAILM: &mut [u32] = unsafe { core::slice::from_raw_parts_mut((&raw mut BUF as *mut u32).add(2), 2) };
struct P(*const [u32]);
unsafe impl Sync for P {}
static RAW: P = P(&raw const BUF as *const [u32; 4] as *const [u32]);
#[unsafe(no_mangle)]
pub fn rust_entry() {
    unsafe {
        show(&*RAW.0);
        BUF[1] = 20;
        show(&*RAW.0);
        show(&*(&raw const *SLM));
        (*(&raw mut SLM))[0] = 100;
        print_u32(BUF[0]);
        (*(&raw mut TAILM))[1] = 400;
        print_u32(BUF[3]);
        BUF[2] = 33;
        print_u32((*(&raw const TAILM))[0]);
        BUF = [7, 8, 9, 10];
        show(&*RAW.0);
        print_u32((*(&raw const TAILM))[0]);
        print_bool(core::ptr::eq(RAW.0 as *const u32, &raw const BUF as *const u32));
        print_bool(core::ptr::eq((*(&raw const TAILM)).as_ptr(), (&raw const BUF as *const u32).add(2)));
        print_usize((*RAW.0).len());
    }
}
'''
P['w06_atomic'] = '''
static CELLS: [AtomicU32; 3] = [AtomicU32::new(1), AtomicU32::new(2), AtomicU32::new(3)];
static SL: &[AtomicU32] = &CELLS;
static TAIL: &[AtomicU32] = CELLS.split_at(1).1;
static OPT: Option<&[AtomicU32]> = Some(&CELLS);
static ONE: AtomicU32 = AtomicU32::new(50);
static ONE_SL: &[AtomicU32] = core::slice::from_ref(&ONE);
struct H { cells: &'static [AtomicU32], tail: Option<&'static [AtomicU32]> }
static HH: H = H { cells: &CELLS, tail: Some(TAIL) };
#[inline(never)] fn total(s: &[AtomicU32]) -> u32 { let mut t = 0; for c in s { t = t * 10 + c.load(Ordering::Relaxed); } t }
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(total(SL)); print_u32(total(TAIL));
    SL[0].store(7, Ordering::Relaxed);
    print_u32(CELLS[0].load(Ordering::Relaxed));
    CELLS[2].store(9, Ordering::Relaxed);
    print_u32(total(TAIL)); print_u32(total(black_box(SL)));
    TAIL[0].fetch_add(1, Ordering::Relaxed);
    print_u32(CELLS[1].load(Ordering::Relaxed));
    print_u32(OPT.map_or(0, total));
    black_box(OPT).unwrap()[1].store(4, Ordering::Relaxed);
    print_u32(total(&CELLS)); print_u32(total(HH.cells)); print_u32(HH.tail.map_or(0, total));
    HH.tail.unwrap()[1].store(6, Ordering::Relaxed);
    print_u32(total(&CELLS));
    ONE_SL[0].store(51, Ordering::Relaxed); print_u32(ONE.load(Ordering::Relaxed));
    ONE.store(52, Ordering::Relaxed); print_u32(total(ONE_SL));
    print_bool(core::ptr::eq(&SL[1], &CELLS[1])); print_bool(core::ptr::eq(TAIL.as_ptr(), &CELLS[1])); print_bool(core::ptr::eq(&ONE_SL[0], &ONE));
}
'''
P['w07_str'] = '''
static BYTES: [u8; 11] = *b"hello world";
static TEXT: &str = match core::str::from_utf8(&BYTES) { Ok(s) => s, Err(_) => "bad" };
static MIDT: &str = match core::str::from_utf8(BYTES.split_at(6).1) { Ok(s) => s, Err(_) => "bad" };
static EMPT: &str = match core::str::from_utf8(BYTES.split_at(11).1) { Ok(s) => s, Err(_) => "bad" };
static OPT: Option<&str> = match core::str::from_utf8(BYTES.split_at(5).0) { Ok(s) => Some(s), Err(_) => None };
static UNI: [u8; 7] = [0xC3, 0xA9, b'x', 0xE2, 0x82, 0xAC, b'!'];
static UTEXT: &str = match core::str::from_utf8(&UNI) { Ok(s) => s, Err(_) => "bad" };
static UTAIL: &str = match core::str::from_utf8(UNI.split_at(3).1) { Ok(s) => s, Err(_) => "bad" };
static BSL: &[u8] = &BYTES;
static BTAIL: &[u8] = BYTES.split_at(6).1;
struct N { name: &'static str, raw: &'static [u8] }
static NN: [N; 2] = [N { name: TEXT, raw: BSL }, N { name: MIDT, raw: BTAIL }];
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_str(TEXT); print_usize(TEXT.len()); print_str(MIDT); print_usize(MIDT.len()); print_str(EMPT); print_usize(EMPT.len());
    print_str(OPT.unwrap_or("none")); print_str(black_box(OPT).unwrap_or("none"));
    print_str(UTEXT); print_usize(UTEXT.len()); print_usize(UTEXT.chars().count()); print_str(UTAIL); print_usize(UTAIL.len());
    print_u32(UTEXT.chars().nth(2).unwrap() as u32);
    print_usize(BSL.len()); print_u32(BSL[4] as u32); print_usize(BTAIL.len()); print_u32(BTAIL[0] as u32);
    print_bool(TEXT.as_bytes() == BSL); print_bool(MIDT.as_bytes() == BTAIL); print_bool(MIDT == "world"); print_bool(black_box(TEXT) == "hello world");
    for n in &NN { print_str(n.name); print_usize(n.raw.len()); print_u32(n.raw[1] as u32); }
    print_str(&TEXT[black_box(2)..5]); print_bool(TEXT.starts_with("hell")); print_usize(TEXT.find('w').unwrap_or(99));
    print_bool(core::ptr::eq(BSL.as_ptr(), BYTES.as_ptr())); print_bool(core::ptr::eq(BTAIL.as_ptr(), &BYTES[6]));
}
'''
P['w07b_str_ptr'] = '''
static BYTES: [u8; 5] = *b"hello";
static TEXT: &str = match core::str::from_utf8(&BYTES) { Ok(s) => s, Err(_) => "bad" };
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_str(TEXT);
    print_bool(core::ptr::eq(TEXT.as_ptr(), BYTES.as_ptr()));
    print_bool(core::ptr::eq(TEXT.as_bytes().as_ptr(), &BYTES[0]));
    print_bool(core::ptr::eq(TEXT.as_bytes(), &BYTES[..]));
}
'''
P['w07c_str_mut'] = '''
static mut BYTES: [u8; 5] = *b"hello";
static mut TEXT: &str = unsafe { core::str::from_utf8_unchecked(&*(&raw const BYTES)) };
#[unsafe(no_mangle)]
pub fn rust_entry() {
    unsafe { print_str(*(&raw const TEXT)); BYTES[0] = b'j'; print_str(*(&raw const TEXT)); }
}
'''
P['w08_agg'] = '''
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
'''
P['w09_holders'] = '''
static ARR: [u32; 4] = [10, 20, 30, 40];
static TAIL: &[u32] = ARR.split_at(2).1;
enum En { Empty, Sl(&'static [u32]), Pair(&'static [u32], u32) }
static ENS: [En; 3] = [En::Sl(&ARR), En::Empty, En::Pair(TAIL, 5)];
static TUP: (&[u32], Option<&[u32]>, u8) = (TAIL, Some(&ARR), 3);
static NEST: &[&[u32]] = &[&ARR, TAIL, &[]];
static ONEST: Option<&[Option<&[u32]>]> = Some(&[Some(&ARR), None, Some(TAIL)]);
static RES: Result<&[u32], &str> = Ok(TAIL);
static RSL: &&[u32] = &TAIL;
#[unsafe(no_mangle)]
pub fn rust_entry() {
    for e in &ENS { match e { En::Empty => print_str("empty"), En::Sl(s) => show(s), En::Pair(s, n) => { show(s); print_u32(*n); } } }
    show(TUP.0); showo(TUP.1); print_u32(TUP.2 as u32);
    for s in NEST { show(s); }
    for o in ONEST.unwrap() { showo(*o); }
    match RES { Ok(s) => show(s), Err(e) => print_str(e) }
    show(*RSL); show(black_box(*RSL));
    print_bool(core::ptr::eq(NEST[1], TAIL)); print_bool(core::ptr::eq(NEST[0].as_ptr(), ARR.as_ptr()));
}
'''
P['w10_runtime'] = '''
static ARR: [u32; 4] = [10, 20, 30, 40];
static SL: &[u32] = &ARR;
static TAIL: &[u32] = ARR.split_at(1).1;
#[inline(never)] fn rt_whole() -> &'static [u32] { &ARR }
#[inline(never)] fn rt_tail(n: usize) -> &'static [u32] { &ARR[n..] }
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let a = rt_whole(); let b = rt_tail(black_box(1));
    show(a); show(b);
    print_bool(core::ptr::eq(a, SL)); print_bool(core::ptr::eq(b, TAIL)); print_bool(a == SL); print_bool(b == TAIL);
    print_bool(core::ptr::eq(a.as_ptr(), SL.as_ptr())); print_bool(core::ptr::eq(b.as_ptr(), TAIL.as_ptr()));
    print_bool(core::ptr::eq(&a[3], &TAIL[2]));
    let o: Option<&[u32]> = Some(rt_tail(black_box(2))); showo(o);
}
'''
P['w11_const_in_body'] = '''
static ARR: [u32; 3] = [100, 200, 300];
const C: &[u32] = &ARR;
#[unsafe(no_mangle)]
pub fn rust_entry() { show(C); print_u32(C[1]); print_bool(core::ptr::eq(C.as_ptr(), ARR.as_ptr())); }
'''
P['w11b_const_opt_in_body'] = '''
static ARR: [u32; 3] = [100, 200, 300];
const CO: Option<&[u32]> = Some(&ARR);
#[unsafe(no_mangle)]
pub fn rust_entry() { showo(CO); showo(black_box(CO)); }
'''
P['w11c_const_str_in_body'] = '''
static BYTES: [u8; 5] = *b"hello";
const CS: &str = match core::str::from_utf8(&BYTES) { Ok(s) => s, Err(_) => "bad" };
#[unsafe(no_mangle)]
pub fn rust_entry() { print_str(CS); print_usize(CS.len()); }
'''
P['w11d_const_via_static'] = '''
static ARR: [u32; 3] = [100, 200, 300];
const C: &[u32] = &ARR;
static S2: &[u32] = C;
struct H { d: &'static [u32] }
const HC: H = H { d: &ARR };
static HS: H = HC;
#[unsafe(no_mangle)]
pub fn rust_entry() { show(S2); show(HS.d); }
'''
P['w11e_const_struct_in_body'] = '''
static ARR: [u32; 3] = [100, 200, 300];
struct H { d: &'static [u32], n: u32 }
const HC: H = H { d: &ARR, n: 4 };
#[unsafe(no_mangle)]
pub fn rust_entry() { let h = HC; show(h.d); print_u32(h.n); }
'''
P['w11f_const_thin_in_body'] = '''
static ARR: [u32; 3] = [100, 200, 300];
const CA: &[u32; 3] = &ARR;
const CO: Option<&[u32; 3]> = Some(&ARR);
#[unsafe(no_mangle)]
pub fn rust_entry() { show(CA); print_u32(CA[2]); let s: &[u32] = CA; show(s); show(CO.unwrap()); print_bool(core::ptr::eq(CA, &ARR)); }
'''
P['w11g_assoc_const'] = '''
static ARR: [u32; 3] = [100, 200, 300];
trait T { const D: &'static [u32]; }
struct A; struct B;
impl T for A { const D: &'static [u32] = &ARR; }
impl T for B { const D: &'static [u32] = ARR.split_at(1).1; }
#[inline(never)] fn g<X: T>() { show(X::D); }
#[unsafe(no_mangle)]
pub fn rust_entry() { g::<A>(); g::<B>(); }
'''
P['w12_promoted_controls'] = '''
const C: &[u32] = &[100, 200, 300];
static S: &[u32] = &[1, 2, 3];
const CS: &str = "hello";
const CO: Option<&[u32]> = Some(&[7, 8]);
static TABLE: [&[u32]; 2] = [&[1], &[2, 3]];
#[unsafe(no_mangle)]
pub fn rust_entry() { show(C); show(S); print_str(CS); showo(CO); show(TABLE[black_box(1)]); print_bool(core::ptr::eq(S, S)); }
'''
P['w13_mut_whole'] = '''
#[derive(Clone, Copy)] struct Pt { x: u32, y: u32 }
static mut ONE: Pt = Pt { x: 1, y: 2 };
static mut ONE_SL: &mut [Pt] = unsafe { core::slice::from_mut(&mut *(&raw mut ONE)) };
static mut V: u32 = 5;
static mut V_SL: &mut [u32] = unsafe { core::slice::from_mut(&mut *(&raw mut V)) };
#[unsafe(no_mangle)]
pub fn rust_entry() {
    unsafe {
        (*(&raw mut ONE_SL))[0].x = 10; print_u32(ONE.x);
        (*(&raw mut ONE_SL))[0] = Pt { x: 7, y: 8 }; print_u32(ONE.x + ONE.y);
        ONE = Pt { x: 20, y: 30 }; print_u32((*(&raw const ONE_SL))[0].x);
        (*(&raw mut V_SL))[0] = 6; print_u32(V);
        V = 9; print_u32((*(&raw const V_SL))[0]);
        print_usize((*(&raw const V_SL)).len());
    }
}
'''
out = sys.argv[1]
os.makedirs(out, exist_ok=True)
for name, body in P.items():
    open(os.path.join(out, name + '.rs'), 'w').write(HEAD + body)
print(len(P))
