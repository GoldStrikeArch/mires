#![no_std]
#![no_main]
#![allow(unused, unreachable_patterns, dead_code)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::cmp::Ordering;
use core::convert::Infallible;
use core::hint::black_box;
use core::marker::PhantomData;
use core::mem::{ManuallyDrop, MaybeUninit};
use core::num::{NonZeroU8, NonZeroU16, NonZeroU32, NonZeroI16, NonZeroU64, NonZeroI8};

trait HasOut { type Out; }
impl<T> HasOut for fn() -> T { type Out = T; }
type Never = <fn() -> ! as HasOut>::Out;

#[derive(Clone, Copy)] enum Void {}
#[derive(Clone, Copy)] struct Wrap<T>(T);
#[derive(Clone, Copy)] #[repr(transparent)] struct Tr<T>(T);
#[derive(Clone, Copy)] #[repr(C)] struct CW<T>(T);
#[derive(Clone, Copy)] #[repr(packed)] struct Pk<T>(T);
#[derive(Clone, Copy)] struct Named<T> { z: (), v: T, p: PhantomData<u64> }
#[derive(Clone, Copy)] struct ZPad<T>([u8; 0], T, ());
#[derive(Clone, Copy)] #[repr(align(4))] struct Aligned(Void);
#[derive(Clone, Copy)] enum AllDead { A(Void), B(Never) }
#[derive(Clone, Copy)] enum DeepDead { A(Wrap<Void>), B((Void,)), C([Void; 1]) }
#[derive(Clone, Copy)] enum HalfDead { A(Void), B }
#[derive(Clone, Copy)] enum HalfDead2 { A, B(Never) }
#[derive(Clone, Copy)] struct NamedDead { a: (), b: Void }
#[derive(Clone, Copy)] #[repr(transparent)] struct TrDead(Void);
#[derive(Clone, Copy)] #[repr(C)] struct CDead(Void);
#[derive(Clone, Copy)] #[repr(packed)] struct PkDead(Void);
#[derive(Clone, Copy)] union UDead { v: Void, u: () }
#[derive(Clone, Copy)] enum SL<T> { Never((), Infallible), Live(T) }
#[derive(Clone, Copy)] enum SL3<T> { D0(Void), D1(Never), Live(T), D3((Void, ())) }

#[derive(Clone, Copy)] #[repr(u8)] enum Small { X = 2, Y = 0xFE }
#[derive(Clone, Copy)] #[repr(u8)] enum Hi { A = 200, B = 253 }
#[derive(Clone, Copy)] #[repr(u8)] enum Hi2 { A = 200, B = 254 }
#[derive(Clone, Copy)] #[repr(u8)] enum Hi3 { A = 250, B = 251, C = 252 }
#[derive(Clone, Copy)] #[repr(i8)] enum Sg { A = -3, B = -1 }
#[derive(Clone, Copy)] #[repr(i8)] enum Sg2 { A = -56, B = -3 }
#[derive(Clone, Copy)] #[repr(i8)] enum Sg3 { A = 100, B = 125 }
#[derive(Clone, Copy)] #[repr(u16)] enum W16 { A = 0xFFF0, B = 0xFFFD }
#[derive(Clone, Copy)] #[repr(i16)] enum S16 { A = -200, B = -3 }
#[derive(Clone, Copy)] #[repr(u32)] enum W32 { A = 0xFFFF_FF00, B = 0xFFFF_FFFD }
#[derive(Clone, Copy)] #[repr(i32)] enum S32 { A = -70000, B = -3 }
#[derive(Clone, Copy)] #[repr(u64)] enum W64 { A = 0xFFFF_FFFF_FFFF_FF00, B = 0xFFFF_FFFF_FFFF_FFFD }
#[derive(Clone, Copy)] #[repr(i64)] enum S64 { A = -5_000_000_000, B = -3 }
#[derive(Clone, Copy)] #[repr(usize)] enum WU { A = 0xFFFF_FF00, B = 0xFFFF_FFFD }
#[derive(Clone, Copy)] enum Three { A, B, C }
#[derive(Clone, Copy)] enum Two { A, B }

static SEVEN: u32 = 77;
fn seven() -> u32 { 7 }
fn eight() -> u32 { 8 }

macro_rules! suite {
    ($id:expr, $f:ident, $T:ty, [$($k:expr),* $(,)?], |$e:ident| $body:expr) => {
        #[inline(never)]
        fn $f() {
            print_i32(-(1000 + $id));
            #[inline(never)]
            fn cls($e: $T) -> u32 { $body }
            #[inline(never)]
            fn cls_ref(r: &$T) -> u32 { let $e = *r; $body }
            #[inline(never)]
            fn cls_o(o: Option<$T>) -> u32 { match o { Some($e) => $body, None => 55555 } }
            #[inline(never)]
            fn cls_oo(o: Option<Option<$T>>) -> u32 { match o { Some(Some($e)) => $body, Some(None) => 55556, None => 55557 } }
            static TABLE: &[$T] = &[$($k),*];
            static OTABLE: [Option<$T>; 2] = [None, None];
            $(
                {
                    const K: $T = $k;
                    static S: $T = $k;
                    { let $e = K; print_u32($body); }
                    print_u32(cls(K));
                    print_u32(cls(black_box(K)));
                    print_u32(cls_ref(&K));
                    print_u32(cls_ref(black_box(&S)));
                    print_u32(cls(S));
                    const O: Option<$T> = Some($k);
                    static SO: Option<$T> = Some($k);
                    print_u32(match O { Some($e) => $body, None => 55555 });
                    print_u32(cls_o(O));
                    print_u32(cls_o(black_box(O)));
                    print_u32(cls_o(SO));
                    const OO: Option<Option<$T>> = Some(Some($k));
                    print_u32(cls_oo(OO));
                    print_u32(cls_oo(black_box(OO)));
                    print_bool(matches!(black_box(OO), Some(Some(_))));
                    print_bool(core::mem::discriminant(&K) == core::mem::discriminant(black_box(&S)));
                }
            )*
            { const N: Option<$T> = None; print_u32(cls_o(N)); print_u32(cls_o(black_box(N))); print_bool(N.is_none()); print_u32(match N { Some($e) => $body, None => 55555 }); }
            { const N: Option<Option<$T>> = Some(None); print_u32(cls_oo(N)); print_u32(cls_oo(black_box(N))); }
            { const N: Option<Option<$T>> = None; print_u32(cls_oo(N)); print_u32(cls_oo(black_box(N))); }
            print_u32(cls_o(OTABLE[black_box(1)]));
            let mut i = 0;
            while i < TABLE.len() {
                print_u32(cls(TABLE[black_box(i)]));
                let mut j = 0;
                while j < TABLE.len() {
                    print_bool(core::mem::discriminant(&TABLE[black_box(i)]) == core::mem::discriminant(&TABLE[j]));
                    j += 1;
                }
                i += 1;
            }
        }
    };
}
#[derive(Clone, Copy)] enum E12 { D1(PhantomData<Void>), Live(SL<NonZeroU32>), D0(Wrap<AllDead>) }
suite!(12, s12, E12, [E12::D1(PhantomData), E12::Live(SL::Live(NonZeroU32::new(1).unwrap())), E12::Live(SL::Live(NonZeroU32::new(0xFFFF_FFFF).unwrap()))], |e| match e { E12::D1(..) => 2002, E12::Live(w) => { let p: NonZeroU32 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); p.get() }, E12::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E13 { U2, D0(DeepDead), U0, U1, Live((W16, PhantomData<Void>, [Void; 0])) }
suite!(13, s13, E13, [E13::U2, E13::U0, E13::U1, E13::Live((W16::A, PhantomData, [])), E13::Live((W16::B, PhantomData, []))], |e| match e { E13::U2 => 1003, E13::D0(..) => 9001, E13::U0 => 1001, E13::U1 => 1002, E13::Live(w) => { let p: W16 = w.0; p as u32 } });
#[derive(Clone, Copy)] enum E14 { D0(Wrap<Void>), U2, U4, Live(CW<S64>), D1(Never), U3, U5, U0, U1 }
suite!(14, s14, E14, [E14::U2, E14::U4, E14::Live(CW(S64::A)), E14::Live(CW(S64::B)), E14::U3, E14::U5, E14::U0, E14::U1], |e| match e { E14::D0(..) => 9001, E14::U2 => 1003, E14::U4 => 1005, E14::Live(w) => { let p: S64 = w.0; (p as i64 as u64 % 1_000_003) as u32 }, E14::D1(..) => 9002, E14::U3 => 1004, E14::U5 => 1006, E14::U0 => 1001, E14::U1 => 1002 });
#[derive(Clone, Copy)] enum E15 { U0, D0(([Void; 0], Void)), Live(Sg3), U1 }
suite!(15, s15, E15, [E15::U0, E15::Live(Sg3::A), E15::Live(Sg3::B), E15::U1], |e| match e { E15::U0 => 1001, E15::D0(..) => 9001, E15::Live(w) => { let p: Sg3 = w; p as i32 as u32 }, E15::U1 => 1002 });
#[derive(Clone, Copy)] enum E16 { D0(Wrap<HalfDead>, ()), Live(Hi), U0 }
suite!(16, s16, E16, [E16::D0(Wrap(HalfDead::B), ()), E16::Live(Hi::A), E16::Live(Hi::B), E16::U0], |e| match e { E16::D0(..) => 2001, E16::Live(w) => { let p: Hi = w; p as u32 }, E16::U0 => 1001 });
#[derive(Clone, Copy)] enum E17 { D2([[Void; 1]; 1]), U1, U0, U3, Live(((), Small)), D1(Wrap<AllDead>), U2, D0(Aligned) }
suite!(17, s17, E17, [E17::U1, E17::U0, E17::U3, E17::Live(((), Small::X)), E17::Live(((), Small::Y)), E17::U2], |e| match e { E17::D2(..) => 9003, E17::U1 => 1002, E17::U0 => 1001, E17::U3 => 1004, E17::Live(w) => { let p: Small = w.1; p as u32 }, E17::D1(..) => 9002, E17::U2 => 1003, E17::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E18 { D2(Never), U1, Live(Wrap<Small>), D0([(Void,); 3]), U0, D1([u32; 0], Void) }
suite!(18, s18, E18, [E18::U1, E18::Live(Wrap(Small::X)), E18::Live(Wrap(Small::Y)), E18::U0], |e| match e { E18::D2(..) => 9003, E18::U1 => 1002, E18::Live(w) => { let p: Small = w.0; p as u32 }, E18::D0(..) => 9001, E18::U0 => 1001, E18::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E19 { D0([(Void,); 3]), U1, Live(CW<bool>), U2, D2(Option<Void>), U0, U3, D1(Wrap<Wrap<(Void, ())>>) }
suite!(19, s19, E19, [E19::U1, E19::Live(CW(true)), E19::Live(CW(false)), E19::U2, E19::D2(None), E19::U0, E19::U3], |e| match e { E19::D0(..) => 9001, E19::U1 => 1002, E19::Live(w) => { let p: bool = w.0; 10 + p as u32 }, E19::U2 => 1003, E19::D2(..) => 2003, E19::U0 => 1001, E19::U3 => 1004, E19::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E20 { D0((Void,)), D2(ManuallyDrop<Void>), Live(SL<WU>), D1([Void; 2]) }
suite!(20, s20, E20, [E20::Live(SL::Live(WU::A)), E20::Live(SL::Live(WU::B))], |e| match e { E20::D0(..) => 9001, E20::D2(..) => 9003, E20::Live(w) => { let p: WU = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); p as usize as u32 }, E20::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E21 { Live(Option<bool>), U1, D0(Void, (), PhantomData<u8>), U0 }
suite!(21, s21, E21, [E21::Live(Some(true)), E21::Live(Some(false)), E21::Live(None), E21::U1, E21::U0], |e| match e { E21::Live(w) => { let p: Option<bool> = w; match p { Some(b) => 40 + b as u32, None => 49 } }, E21::U1 => 1002, E21::D0(..) => 9001, E21::U0 => 1001 });
#[derive(Clone, Copy)] enum E22 { D1(Void, [u64; 0]), D0([(Void,); 0]), Live(ManuallyDrop<char>) }
suite!(22, s22, E22, [E22::D0([]), E22::Live(ManuallyDrop::new('a')), E22::Live(ManuallyDrop::new('\u{10FFFF}')), E22::Live(ManuallyDrop::new('\0'))], |e| match e { E22::D1(..) => 9002, E22::D0(..) => 2001, E22::Live(w) => { let p: char = ManuallyDrop::into_inner(w); p as u32 } });
#[derive(Clone, Copy)] enum E23 { D0(Result<(), Void>), U0, U2, Live(S16), U1 }
suite!(23, s23, E23, [E23::D0(Ok(())), E23::U0, E23::U2, E23::Live(S16::A), E23::Live(S16::B), E23::U1], |e| match e { E23::D0(..) => 2001, E23::U0 => 1001, E23::U2 => 1003, E23::Live(w) => { let p: S16 = w; p as i32 as u32 }, E23::U1 => 1002 });

#[unsafe(no_mangle)]
pub fn rust_entry() {
    s12(); s13(); s14(); s15(); s16(); s17(); s18(); s19(); s20(); s21(); s22(); s23();
}
