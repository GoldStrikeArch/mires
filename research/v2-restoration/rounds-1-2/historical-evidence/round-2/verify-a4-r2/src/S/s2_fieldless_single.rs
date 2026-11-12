#![no_std]
#![no_main]
#![allow(unused, unreachable_patterns, dead_code)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::hint::black_box;
use core::mem::discriminant;

#[derive(Clone, Copy, PartialEq)] enum One { A }
#[derive(Clone, Copy, PartialEq)] enum OneNeg { A = -5 }
#[derive(Clone, Copy, PartialEq)] #[repr(u8)] enum OneU8 { A = 7 }
#[derive(Clone, Copy, PartialEq)] #[repr(i16)] enum OneI16 { A = -300 }
#[derive(Clone, Copy, PartialEq)] #[repr(C)] enum OneC { A = 9 }
#[derive(Clone, Copy, PartialEq)] #[repr(u8)] enum TwoU8 { A = 200, B = 254 }
#[derive(Clone, Copy)] #[repr(u8)] enum DataU8 { A(u8) = 3, B = 9 }
#[derive(Clone, Copy)] #[repr(C)] enum DataC { A(u8), B }
#[derive(Clone, Copy)] #[repr(u8)] enum OneData { A(char) = 4 }
#[derive(Clone, Copy)] #[repr(C, u8)] enum CU8 { A(u32) = 1, B = 2 }

#[inline(never)] fn opt<T: Copy>(o: Option<T>) -> u32 { if o.is_some() { 1 } else { 0 } }

#[unsafe(no_mangle)]
pub fn rust_entry() {
    const K1: One = One::A; const K2: OneNeg = OneNeg::A; const K3: OneU8 = OneU8::A; const K4: OneI16 = OneI16::A; const K5: OneC = OneC::A;
    print_i32(K1 as i32); print_i32(K2 as i32); print_i32(K3 as i32); print_i32(K4 as i32); print_i32(K5 as i32);
    print_i32(black_box(K1) as i32); print_i32(black_box(K2) as i32); print_i32(black_box(K3) as i32); print_i32(black_box(K4) as i32); print_i32(black_box(K5) as i32);
    print_bool(black_box(K1) == One::A); print_bool(black_box(K3) == OneU8::A); print_bool(discriminant(&K2) == discriminant(&black_box(OneNeg::A)));
    // Option of each: which have a niche, and where it sits, is rustc's business
    const O1: Option<One> = Some(One::A); const N1: Option<One> = None;
    const O2: Option<OneNeg> = Some(OneNeg::A); const N2: Option<OneNeg> = None;
    const O3: Option<OneU8> = Some(OneU8::A); const N3: Option<OneU8> = None;
    const O4: Option<OneI16> = Some(OneI16::A); const N4: Option<OneI16> = None;
    const O5: Option<OneC> = Some(OneC::A); const N5: Option<OneC> = None;
    print_u32(opt(O1)); print_u32(opt(N1)); print_u32(opt(black_box(O1))); print_u32(opt(black_box(N1)));
    print_u32(opt(O2)); print_u32(opt(N2)); print_u32(opt(black_box(O2))); print_u32(opt(black_box(N2)));
    print_u32(opt(O3)); print_u32(opt(N3)); print_u32(opt(black_box(O3))); print_u32(opt(black_box(N3)));
    print_u32(opt(O4)); print_u32(opt(N4)); print_u32(opt(black_box(O4))); print_u32(opt(black_box(N4)));
    print_u32(opt(O5)); print_u32(opt(N5)); print_u32(opt(black_box(O5))); print_u32(opt(black_box(N5)));
    print_i32(match black_box(O3) { Some(v) => v as i32, None => -1 });
    print_i32(match black_box(O4) { Some(v) => v as i32, None => -1 });
    print_i32(match black_box(O5) { Some(v) => v as i32, None => -1 });
    const OO3: Option<Option<OneU8>> = Some(None); const OO3S: Option<Option<OneU8>> = Some(Some(OneU8::A)); const OO3N: Option<Option<OneU8>> = None;
    print_u32(match black_box(OO3) { Some(Some(_)) => 2, Some(None) => 1, None => 0 });
    print_u32(match black_box(OO3S) { Some(Some(v)) => v as u32, Some(None) => 1, None => 0 });
    print_u32(match black_box(OO3N) { Some(Some(_)) => 2, Some(None) => 1, None => 0 });
    // wrapping niche in a repr(u8) enum: 255, then 0
    const T0: Option<Option<TwoU8>> = Some(Some(TwoU8::B)); const T1: Option<Option<TwoU8>> = Some(None); const T2: Option<Option<TwoU8>> = None;
    print_u32(match black_box(T0) { Some(Some(v)) => v as u32, Some(None) => 1, None => 0 });
    print_u32(match black_box(T1) { Some(Some(v)) => v as u32, Some(None) => 1, None => 0 });
    print_u32(match black_box(T2) { Some(Some(v)) => v as u32, Some(None) => 1, None => 0 });
    print_u32(match T1 { Some(Some(v)) => v as u32, Some(None) => 1, None => 0 });
    print_u32(match T2 { Some(Some(v)) => v as u32, Some(None) => 1, None => 0 });
    // explicit-repr enums with data
    const D1: DataU8 = DataU8::A(5); const D2: DataU8 = DataU8::B;
    print_u32(match black_box(D1) { DataU8::A(n) => n as u32, DataU8::B => 100 }); print_u32(match black_box(D2) { DataU8::A(n) => n as u32, DataU8::B => 100 });
    const OD1: Option<DataU8> = Some(DataU8::B); const OD2: Option<DataU8> = None; const OD3: Option<DataU8> = Some(DataU8::A(6));
    print_u32(match black_box(OD1) { Some(DataU8::A(n)) => n as u32, Some(DataU8::B) => 100, None => 0 });
    print_u32(match black_box(OD2) { Some(DataU8::A(n)) => n as u32, Some(DataU8::B) => 100, None => 0 });
    print_u32(match black_box(OD3) { Some(DataU8::A(n)) => n as u32, Some(DataU8::B) => 100, None => 0 });
    const C1: DataC = DataC::A(8); const OC1: Option<DataC> = Some(DataC::B); const OC2: Option<DataC> = None;
    print_u32(match black_box(C1) { DataC::A(n) => n as u32, DataC::B => 100 });
    print_u32(match black_box(OC1) { Some(DataC::A(n)) => n as u32, Some(DataC::B) => 100, None => 0 });
    print_u32(match black_box(OC2) { Some(DataC::A(n)) => n as u32, Some(DataC::B) => 100, None => 0 });
    const E1: OneData = OneData::A('e'); const OE1: Option<OneData> = Some(OneData::A('f')); const OE2: Option<OneData> = None;
    print_u32(match black_box(E1) { OneData::A(c) => c as u32 });
    print_u32(match black_box(OE1) { Some(OneData::A(c)) => c as u32, None => 0 });
    print_u32(match black_box(OE2) { Some(OneData::A(c)) => c as u32, None => 0 });
    print_u32(match OE1 { Some(OneData::A(c)) => c as u32, None => 0 });
    const U1: CU8 = CU8::A(11); const OU1: Option<CU8> = Some(CU8::B); const OU2: Option<CU8> = None;
    print_u32(match black_box(U1) { CU8::A(n) => n, CU8::B => 100 });
    print_u32(match black_box(OU1) { Some(CU8::A(n)) => n, Some(CU8::B) => 100, None => 0 });
    print_u32(match black_box(OU2) { Some(CU8::A(n)) => n, Some(CU8::B) => 100, None => 0 });
}
