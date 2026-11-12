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
    ($id:expr, $f:ident, $T:ty, [$($k:expr),* $(,)?], [$($r:expr),* $(,)?], |$e:ident| $body:expr) => {
        #[inline(never)]
        fn $f() {
            print_i32(-(1000 + $id));
            // the same values built at run time
            $(
                {
                    #[inline(never)]
                    fn rt_cls($e: $T) -> u32 { $body }
                    #[inline(never)]
                    fn mk() -> $T { $r }
                    { let $e = $r; print_u32($body); }
                    print_u32(rt_cls($r));
                    print_u32(rt_cls(mk()));
                    let o = Some(mk());
                    print_u32(match black_box(o) { Some($e) => $body, None => 55555 });
                    let oo = black_box(Some(Some($r)));
                    print_u32(match oo { Some(Some($e)) => $body, Some(None) => 55556, None => 55557 });
                    print_bool(core::mem::discriminant(&mk()) == core::mem::discriminant(&$r));
                }
            )*
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
#[derive(Clone, Copy)] enum E20 { D0((Never, Never)), U0, Live(((), Sg3)), U1 }
suite!(20, s20, E20, [E20::U0, E20::Live(((), Sg3::A)), E20::Live(((), Sg3::B)), E20::U1], [E20::Live(((), black_box(Sg3::A))), E20::Live(((), black_box(Sg3::B)))], |e| match e { E20::D0(..) => 9001, E20::U0 => 1001, E20::Live(w) => { let p: Sg3 = w.1; p as i32 as u32 }, E20::U1 => 1002 });
#[derive(Clone, Copy)] enum E21 { U0, U1, Live(SL<Wrap<SL3<Option<bool>>>>), D0(Option<Void>) }
suite!(21, s21, E21, [E21::U0, E21::U1, E21::Live(SL::Live(Wrap(SL3::Live(Some(true))))), E21::Live(SL::Live(Wrap(SL3::Live(Some(false))))), E21::Live(SL::Live(Wrap(SL3::Live(None)))), E21::D0(None)], [E21::Live(SL::Live(Wrap(SL3::Live(black_box(Some(true)))))), E21::Live(SL::Live(Wrap(SL3::Live(black_box(Some(false)))))), E21::Live(SL::Live(Wrap(SL3::Live(black_box(None)))))], |e| match e { E21::U0 => 1001, E21::U1 => 1002, E21::Live(w) => { let p: Option<bool> = (match w { SL::Live(Wrap(SL3::Live(x))) => x, _ => loop {} }); match p { Some(b) => 40 + b as u32, None => 49 } }, E21::D0(..) => 2001 });
#[derive(Clone, Copy)] enum E22 { D0(u8, Void), D1((HalfDead, Void)), Live(Tr<WU>), U1, U0 }
suite!(22, s22, E22, [E22::Live(Tr(WU::A)), E22::Live(Tr(WU::B)), E22::U1, E22::U0], [E22::Live(Tr(black_box(WU::A))), E22::Live(Tr(black_box(WU::B)))], |e| match e { E22::D0(..) => 9001, E22::D1(..) => 9002, E22::Live(w) => { let p: WU = w.0; p as usize as u32 }, E22::U1 => 1002, E22::U0 => 1001 });
#[derive(Clone, Copy)] enum E23 { Live(Wrap<S64>), D1(DeepDead), D0([u16; 0], Void) }
suite!(23, s23, E23, [E23::Live(Wrap(S64::A)), E23::Live(Wrap(S64::B))], [E23::Live(Wrap(black_box(S64::A))), E23::Live(Wrap(black_box(S64::B)))], |e| match e { E23::Live(w) => { let p: S64 = w.0; (p as i64 as u64 % 1_000_003) as u32 }, E23::D1(..) => 9002, E23::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E24 { D0(Option<Void>), Live(CW<&'static u32>) }
suite!(24, s24, E24, [E24::D0(None), E24::Live(CW(&5)), E24::Live(CW(&SEVEN))], [E24::Live(CW(black_box(&5))), E24::Live(CW(black_box(&SEVEN)))], |e| match e { E24::D0(..) => 2001, E24::Live(w) => { let p: &'static u32 = w.0; *p } });
#[derive(Clone, Copy)] enum E25 { U0, Live(ZPad<Sg3>) }
suite!(25, s25, E25, [E25::U0, E25::Live(ZPad([], Sg3::A, ())), E25::Live(ZPad([], Sg3::B, ()))], [E25::Live(ZPad([], black_box(Sg3::A), ())), E25::Live(ZPad([], black_box(Sg3::B), ()))], |e| match e { E25::U0 => 1001, E25::Live(w) => { let p: Sg3 = w.1; p as i32 as u32 } });
#[derive(Clone, Copy)] enum E26 { D0([(Void,); 0]), Live(Pk<S16>), D1(([Void; 0], Void)), U1, U0 }
suite!(26, s26, E26, [E26::D0([]), E26::Live(Pk(S16::A)), E26::Live(Pk(S16::B)), E26::U1, E26::U0], [E26::Live(Pk(black_box(S16::A))), E26::Live(Pk(black_box(S16::B)))], |e| match e { E26::D0(..) => 2001, E26::Live(w) => { let p: S16 = { w.0 }; p as i32 as u32 }, E26::D1(..) => 9002, E26::U1 => 1002, E26::U0 => 1001 });
#[derive(Clone, Copy)] enum E27 { Live(Tr<S32>), U2, U0, U1, D0(UDead) }
suite!(27, s27, E27, [E27::Live(Tr(S32::A)), E27::Live(Tr(S32::B)), E27::U2, E27::U0, E27::U1], [E27::Live(Tr(black_box(S32::A))), E27::Live(Tr(black_box(S32::B)))], |e| match e { E27::Live(w) => { let p: S32 = w.0; p as i32 as u32 }, E27::U2 => 1003, E27::U0 => 1001, E27::U1 => 1002, E27::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E28 { U0, U1, D0(Void), D1(&'static Void, Void), Live(((), Sg2)), U2 }
suite!(28, s28, E28, [E28::U0, E28::U1, E28::Live(((), Sg2::A)), E28::Live(((), Sg2::B)), E28::U2], [E28::Live(((), black_box(Sg2::A))), E28::Live(((), black_box(Sg2::B)))], |e| match e { E28::U0 => 1001, E28::U1 => 1002, E28::D0(..) => 9001, E28::D1(..) => 9002, E28::Live(w) => { let p: Sg2 = w.1; p as i32 as u32 }, E28::U2 => 1003 });
#[derive(Clone, Copy)] enum E29 { U5, U0, U3, U1, D0(DeepDead), U4, Live(SL<Wrap<SL3<Option<Option<Three>>>>>), U2, D1([Void; 2]) }
suite!(29, s29, E29, [E29::U5, E29::U0, E29::U3, E29::U1, E29::U4, E29::Live(SL::Live(Wrap(SL3::Live(Some(Some(Three::C)))))), E29::Live(SL::Live(Wrap(SL3::Live(Some(None))))), E29::Live(SL::Live(Wrap(SL3::Live(None)))), E29::U2], [E29::Live(SL::Live(Wrap(SL3::Live(black_box(Some(Some(Three::C))))))), E29::Live(SL::Live(Wrap(SL3::Live(black_box(Some(None)))))), E29::Live(SL::Live(Wrap(SL3::Live(black_box(None)))))], |e| match e { E29::U5 => 1006, E29::U0 => 1001, E29::U3 => 1004, E29::U1 => 1002, E29::D0(..) => 9001, E29::U4 => 1005, E29::Live(w) => { let p: Option<Option<Three>> = (match w { SL::Live(Wrap(SL3::Live(x))) => x, _ => loop {} }); match p { Some(Some(t)) => t as u32 + 60, Some(None) => 46, None => 45 } }, E29::U2 => 1003, E29::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E30 { U0, U1, Live(Wrap<Tr<((), Option<bool>)>>) }
suite!(30, s30, E30, [E30::U0, E30::U1, E30::Live(Wrap(Tr(((), Some(true))))), E30::Live(Wrap(Tr(((), Some(false))))), E30::Live(Wrap(Tr(((), None))))], [E30::Live(Wrap(Tr(((), black_box(Some(true)))))), E30::Live(Wrap(Tr(((), black_box(Some(false)))))), E30::Live(Wrap(Tr(((), black_box(None)))))], |e| match e { E30::U0 => 1001, E30::U1 => 1002, E30::Live(w) => { let p: Option<bool> = w.0.0.1; match p { Some(b) => 40 + b as u32, None => 49 } } });
#[derive(Clone, Copy)] enum E31 { D0((), Infallible), U0, D1(Void, [u64; 0]), Live(Result<Never, bool>) }
suite!(31, s31, E31, [E31::U0, E31::Live(Err(true)), E31::Live(Err(false))], [E31::Live(Err(black_box(true))), E31::Live(Err(black_box(false)))], |e| match e { E31::D0(..) => 9001, E31::U0 => 1001, E31::D1(..) => 9002, E31::Live(w) => { let p: bool = (match w { Err(x) => x, Ok(_) => loop {} }); 10 + p as u32 } });
#[derive(Clone, Copy)] enum E32 { U0, D0([[Void; 1]; 1]), D2(Void, (), PhantomData<u8>), D1(TrDead), Live(((), S16)) }
suite!(32, s32, E32, [E32::U0, E32::Live(((), S16::A)), E32::Live(((), S16::B))], [E32::Live(((), black_box(S16::A))), E32::Live(((), black_box(S16::B)))], |e| match e { E32::U0 => 1001, E32::D0(..) => 9001, E32::D2(..) => 9003, E32::D1(..) => 9002, E32::Live(w) => { let p: S16 = w.1; p as i32 as u32 } });
#[derive(Clone, Copy)] enum E33 { U2, U3, Live(ManuallyDrop<Option<bool>>), U0, U1 }
suite!(33, s33, E33, [E33::U2, E33::U3, E33::Live(ManuallyDrop::new(Some(true))), E33::Live(ManuallyDrop::new(Some(false))), E33::Live(ManuallyDrop::new(None)), E33::U0, E33::U1], [E33::Live(ManuallyDrop::new(black_box(Some(true)))), E33::Live(ManuallyDrop::new(black_box(Some(false)))), E33::Live(ManuallyDrop::new(black_box(None)))], |e| match e { E33::U2 => 1003, E33::U3 => 1004, E33::Live(w) => { let p: Option<bool> = ManuallyDrop::into_inner(w); match p { Some(b) => 40 + b as u32, None => 49 } }, E33::U0 => 1001, E33::U1 => 1002 });
#[derive(Clone, Copy)] enum E34 { D0(Void), Live(Wrap<W32>), U0 }
suite!(34, s34, E34, [E34::Live(Wrap(W32::A)), E34::Live(Wrap(W32::B)), E34::U0], [E34::Live(Wrap(black_box(W32::A))), E34::Live(Wrap(black_box(W32::B)))], |e| match e { E34::D0(..) => 9001, E34::Live(w) => { let p: W32 = w.0; p as u32 }, E34::U0 => 1001 });
#[derive(Clone, Copy)] enum E35 { U2, U0, Live(Wrap<Tr<((), W16)>>), U1 }
suite!(35, s35, E35, [E35::U2, E35::U0, E35::Live(Wrap(Tr(((), W16::A)))), E35::Live(Wrap(Tr(((), W16::B)))), E35::U1], [E35::Live(Wrap(Tr(((), black_box(W16::A))))), E35::Live(Wrap(Tr(((), black_box(W16::B)))))], |e| match e { E35::U2 => 1003, E35::U0 => 1001, E35::Live(w) => { let p: W16 = w.0.0.1; p as u32 }, E35::U1 => 1002 });
#[derive(Clone, Copy)] enum E36 { U2, U5, U3, U0, U4, D1((Void,)), U1, D0([Void; 2]), Live(Result<Never, W64>) }
suite!(36, s36, E36, [E36::U2, E36::U5, E36::U3, E36::U0, E36::U4, E36::U1, E36::Live(Err(W64::A)), E36::Live(Err(W64::B))], [E36::Live(Err(black_box(W64::A))), E36::Live(Err(black_box(W64::B)))], |e| match e { E36::U2 => 1003, E36::U5 => 1006, E36::U3 => 1004, E36::U0 => 1001, E36::U4 => 1005, E36::D1(..) => 9002, E36::U1 => 1002, E36::D0(..) => 9001, E36::Live(w) => { let p: W64 = (match w { Err(x) => x, Ok(_) => loop {} }); (p as u64 % 1_000_003) as u32 } });
#[derive(Clone, Copy)] enum E37 { Live(W16), D0(Wrap<Wrap<(Void, ())>>), D1(Void, [u64; 0]) }
suite!(37, s37, E37, [E37::Live(W16::A), E37::Live(W16::B)], [E37::Live(black_box(W16::A)), E37::Live(black_box(W16::B))], |e| match e { E37::Live(w) => { let p: W16 = w; p as u32 }, E37::D0(..) => 9001, E37::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E38 { Live(Hi2), D1(Aligned), U0, U1, D0([[Void; 1]; 1]) }
suite!(38, s38, E38, [E38::Live(Hi2::A), E38::Live(Hi2::B), E38::U0, E38::U1], [E38::Live(black_box(Hi2::A)), E38::Live(black_box(Hi2::B))], |e| match e { E38::Live(w) => { let p: Hi2 = w; p as u32 }, E38::D1(..) => 9002, E38::U0 => 1001, E38::U1 => 1002, E38::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E39 { D1(Wrap<Void>), U0, D0(Option<Void>), Live(Wrap<Sg2>) }
suite!(39, s39, E39, [E39::U0, E39::D0(None), E39::Live(Wrap(Sg2::A)), E39::Live(Wrap(Sg2::B))], [E39::Live(Wrap(black_box(Sg2::A))), E39::Live(Wrap(black_box(Sg2::B)))], |e| match e { E39::D1(..) => 9002, E39::U0 => 1001, E39::D0(..) => 2001, E39::Live(w) => { let p: Sg2 = w.0; p as i32 as u32 } });

#[unsafe(no_mangle)]
pub fn rust_entry() {
    s20(); s21(); s22(); s23(); s24(); s25(); s26(); s27(); s28(); s29(); s30(); s31(); s32(); s33(); s34(); s35(); s36(); s37(); s38(); s39();
}
