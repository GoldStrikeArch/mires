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
#[derive(Clone, Copy)] enum E20 { U0, U5, U3, U1, D2([(Void,); 0]), D0([(Void,); 0]), U4, D1((Never, Never)), U2, Live(Named<S64>) }
suite!(20, s20, E20, [E20::U0, E20::U5, E20::U3, E20::U1, E20::D2([]), E20::D0([]), E20::U4, E20::U2, E20::Live(Named { z: (), v: S64::A, p: PhantomData }), E20::Live(Named { z: (), v: S64::B, p: PhantomData })], |e| match e { E20::U0 => 1001, E20::U5 => 1006, E20::U3 => 1004, E20::U1 => 1002, E20::D2(..) => 2003, E20::D0(..) => 2001, E20::U4 => 1005, E20::D1(..) => 9002, E20::U2 => 1003, E20::Live(w) => { let p: S64 = w.v; (p as i64 as u64 % 1_000_003) as u32 } });
#[derive(Clone, Copy)] enum E21 { U1, D0(Wrap<AllDead>), D1(Void, [u64; 0]), U0, Live(SL<Small>) }
suite!(21, s21, E21, [E21::U1, E21::U0, E21::Live(SL::Live(Small::X)), E21::Live(SL::Live(Small::Y))], |e| match e { E21::U1 => 1002, E21::D0(..) => 9001, E21::D1(..) => 9002, E21::U0 => 1001, E21::Live(w) => { let p: Small = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); p as u32 } });
#[derive(Clone, Copy)] enum E22 { D0((Void, ())), Live(Result<Never, NonZeroI8>) }
suite!(22, s22, E22, [E22::Live(Err(NonZeroI8::new(-1).unwrap())), E22::Live(Err(NonZeroI8::new(127).unwrap()))], |e| match e { E22::D0(..) => 9001, E22::Live(w) => { let p: NonZeroI8 = (match w { Err(x) => x, Ok(_) => loop {} }); p.get() as i32 as u32 } });
#[derive(Clone, Copy)] enum E23 { U0, Live(CW<char>) }
suite!(23, s23, E23, [E23::U0, E23::Live(CW('a')), E23::Live(CW('\u{10FFFF}')), E23::Live(CW('\0'))], |e| match e { E23::U0 => 1001, E23::Live(w) => { let p: char = w.0; p as u32 } });
#[derive(Clone, Copy)] enum E24 { D0((HalfDead, Void)), Live(Result<Never, char>), D1((HalfDead, Void)), U0, U2, U3, U1 }
suite!(24, s24, E24, [E24::Live(Err('a')), E24::Live(Err('\u{10FFFF}')), E24::Live(Err('\0')), E24::U0, E24::U2, E24::U3, E24::U1], |e| match e { E24::D0(..) => 9001, E24::Live(w) => { let p: char = (match w { Err(x) => x, Ok(_) => loop {} }); p as u32 }, E24::D1(..) => 9002, E24::U0 => 1001, E24::U2 => 1003, E24::U3 => 1004, E24::U1 => 1002 });
#[derive(Clone, Copy)] enum E25 { D0((Void, ())), Live(S16), D1(Wrap<HalfDead>, ()) }
suite!(25, s25, E25, [E25::Live(S16::A), E25::Live(S16::B), E25::D1(Wrap(HalfDead::B), ())], |e| match e { E25::D0(..) => 9001, E25::Live(w) => { let p: S16 = w; p as i32 as u32 }, E25::D1(..) => 2002 });
#[derive(Clone, Copy)] enum E26 { U0, Live(Result<&'static u32, Void>), U1 }
suite!(26, s26, E26, [E26::U0, E26::Live(Ok(&5)), E26::Live(Ok(&SEVEN)), E26::U1], |e| match e { E26::U0 => 1001, E26::Live(w) => { let p: &'static u32 = (match w { Ok(x) => x, Err(_) => loop {} }); *p }, E26::U1 => 1002 });
#[derive(Clone, Copy)] enum E27 { D0((Void, ())), U0, D1((Void, ())), Live((W64, PhantomData<Void>, [Void; 0])), D2(ManuallyDrop<Void>), U1 }
suite!(27, s27, E27, [E27::U0, E27::Live((W64::A, PhantomData, [])), E27::Live((W64::B, PhantomData, [])), E27::U1], |e| match e { E27::D0(..) => 9001, E27::U0 => 1001, E27::D1(..) => 9002, E27::Live(w) => { let p: W64 = w.0; (p as u64 % 1_000_003) as u32 }, E27::D2(..) => 9003, E27::U1 => 1002 });
#[derive(Clone, Copy)] enum E28 { U1, Live(NonZeroU32), U0, U3, U2 }
suite!(28, s28, E28, [E28::U1, E28::Live(NonZeroU32::new(1).unwrap()), E28::Live(NonZeroU32::new(0xFFFF_FFFF).unwrap()), E28::U0, E28::U3, E28::U2], |e| match e { E28::U1 => 1002, E28::Live(w) => { let p: NonZeroU32 = w; p.get() }, E28::U0 => 1001, E28::U3 => 1004, E28::U2 => 1003 });
#[derive(Clone, Copy)] enum E29 { Live(Wrap<Tr<((), Option<char>)>>) }
suite!(29, s29, E29, [E29::Live(Wrap(Tr(((), Some('z'))))), E29::Live(Wrap(Tr(((), None))))], |e| match e { E29::Live(w) => { let p: Option<char> = w.0.0.1; match p { Some(c) => c as u32, None => 48 } } });
#[derive(Clone, Copy)] enum E30 { U1, U0, D0(Aligned), Live(Hi) }
suite!(30, s30, E30, [E30::U1, E30::U0, E30::Live(Hi::A), E30::Live(Hi::B)], |e| match e { E30::U1 => 1002, E30::U0 => 1001, E30::D0(..) => 9001, E30::Live(w) => { let p: Hi = w; p as u32 } });
#[derive(Clone, Copy)] enum E31 { Live(Wrap<Tr<((), NonZeroI8)>>), U1, D1((), Infallible), U0, D0(TrDead) }
suite!(31, s31, E31, [E31::Live(Wrap(Tr(((), NonZeroI8::new(-1).unwrap())))), E31::Live(Wrap(Tr(((), NonZeroI8::new(127).unwrap())))), E31::U1, E31::U0], |e| match e { E31::Live(w) => { let p: NonZeroI8 = w.0.0.1; p.get() as i32 as u32 }, E31::U1 => 1002, E31::D1(..) => 9002, E31::U0 => 1001, E31::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E32 { Live(Wrap<NonZeroI8>), D1(AllDead), D0((Void, ())) }
suite!(32, s32, E32, [E32::Live(Wrap(NonZeroI8::new(-1).unwrap())), E32::Live(Wrap(NonZeroI8::new(127).unwrap()))], |e| match e { E32::Live(w) => { let p: NonZeroI8 = w.0; p.get() as i32 as u32 }, E32::D1(..) => 9002, E32::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E33 { Live(SL<Wrap<SL3<char>>>), U3, U0, U1, U2 }
suite!(33, s33, E33, [E33::Live(SL::Live(Wrap(SL3::Live('a')))), E33::Live(SL::Live(Wrap(SL3::Live('\u{10FFFF}')))), E33::Live(SL::Live(Wrap(SL3::Live('\0')))), E33::U3, E33::U0, E33::U1, E33::U2], |e| match e { E33::Live(w) => { let p: char = (match w { SL::Live(Wrap(SL3::Live(x))) => x, _ => loop {} }); p as u32 }, E33::U3 => 1004, E33::U0 => 1001, E33::U1 => 1002, E33::U2 => 1003 });
#[derive(Clone, Copy)] enum E34 { Live((NonZeroU16, PhantomData<Void>, [Void; 0])), U0, U1, D0([Void; 0]), D1(PkDead) }
suite!(34, s34, E34, [E34::Live((NonZeroU16::new(1).unwrap(), PhantomData, [])), E34::Live((NonZeroU16::new(65535).unwrap(), PhantomData, [])), E34::U0, E34::U1, E34::D0([])], |e| match e { E34::Live(w) => { let p: NonZeroU16 = w.0; p.get() as u32 }, E34::U0 => 1001, E34::U1 => 1002, E34::D0(..) => 2001, E34::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E35 { Live(Named<Sg>), U0, D1(Wrap<Void>), D0(Result<(), Void>) }
suite!(35, s35, E35, [E35::Live(Named { z: (), v: Sg::A, p: PhantomData }), E35::Live(Named { z: (), v: Sg::B, p: PhantomData }), E35::U0, E35::D0(Ok(()))], |e| match e { E35::Live(w) => { let p: Sg = w.v; p as i32 as u32 }, E35::U0 => 1001, E35::D1(..) => 9002, E35::D0(..) => 2001 });
#[derive(Clone, Copy)] enum E36 { Live(Wrap<Tr<((), Two)>>) }
suite!(36, s36, E36, [E36::Live(Wrap(Tr(((), Two::A)))), E36::Live(Wrap(Tr(((), Two::B))))], |e| match e { E36::Live(w) => { let p: Two = w.0.0.1; p as u32 + 30 } });
#[derive(Clone, Copy)] enum E37 { U3, U1, U2, U0, U4, D0(DeepDead), Live(CW<Hi3>), U5 }
suite!(37, s37, E37, [E37::U3, E37::U1, E37::U2, E37::U0, E37::U4, E37::Live(CW(Hi3::A)), E37::Live(CW(Hi3::B)), E37::Live(CW(Hi3::C)), E37::U5], |e| match e { E37::U3 => 1004, E37::U1 => 1002, E37::U2 => 1003, E37::U0 => 1001, E37::U4 => 1005, E37::D0(..) => 9001, E37::Live(w) => { let p: Hi3 = w.0; p as u32 }, E37::U5 => 1006 });
#[derive(Clone, Copy)] enum E38 { D0((Void, ())), U0, U2, Live((Three,)), U1, U3 }
suite!(38, s38, E38, [E38::U0, E38::U2, E38::Live((Three::A,)), E38::Live((Three::B,)), E38::Live((Three::C,)), E38::U1, E38::U3], |e| match e { E38::D0(..) => 9001, E38::U0 => 1001, E38::U2 => 1003, E38::Live(w) => { let p: Three = w.0; p as u32 + 20 }, E38::U1 => 1002, E38::U3 => 1004 });
#[derive(Clone, Copy)] enum E39 { U0, D1((u32, Void)), Live(Result<Never, Hi2>), D0(NamedDead), D2([(Void,); 3]) }
suite!(39, s39, E39, [E39::U0, E39::Live(Err(Hi2::A)), E39::Live(Err(Hi2::B))], |e| match e { E39::U0 => 1001, E39::D1(..) => 9002, E39::Live(w) => { let p: Hi2 = (match w { Err(x) => x, Ok(_) => loop {} }); p as u32 }, E39::D0(..) => 9001, E39::D2(..) => 9003 });

#[unsafe(no_mangle)]
pub fn rust_entry() {
    s20(); s21(); s22(); s23(); s24(); s25(); s26(); s27(); s28(); s29(); s30(); s31(); s32(); s33(); s34(); s35(); s36(); s37(); s38(); s39();
}
