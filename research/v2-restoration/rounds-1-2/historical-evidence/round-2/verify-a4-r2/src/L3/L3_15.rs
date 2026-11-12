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
#[derive(Clone, Copy)] enum E300 { D1(TrDead), U0, Live(NonZeroU32), D0(Wrap<Void>), U1 }
suite!(300, s300, E300, [E300::U0, E300::Live(NonZeroU32::new(1).unwrap()), E300::Live(NonZeroU32::new(0xFFFF_FFFF).unwrap()), E300::U1], [E300::Live(black_box(NonZeroU32::new(1).unwrap())), E300::Live(black_box(NonZeroU32::new(0xFFFF_FFFF).unwrap()))], |e| match e { E300::D1(..) => 9002, E300::U0 => 1001, E300::Live(w) => { let p: NonZeroU32 = w; p.get() }, E300::D0(..) => 9001, E300::U1 => 1002 });
#[derive(Clone, Copy)] enum E301 { D1([Void; 2]), D0([(Void,); 0]), U0, Live(Wrap<Tr<((), Three)>>) }
suite!(301, s301, E301, [E301::D0([]), E301::U0, E301::Live(Wrap(Tr(((), Three::A)))), E301::Live(Wrap(Tr(((), Three::B)))), E301::Live(Wrap(Tr(((), Three::C))))], [E301::Live(Wrap(Tr(((), black_box(Three::A))))), E301::Live(Wrap(Tr(((), black_box(Three::B))))), E301::Live(Wrap(Tr(((), black_box(Three::C)))))], |e| match e { E301::D1(..) => 9002, E301::D0(..) => 2001, E301::U0 => 1001, E301::Live(w) => { let p: Three = w.0.0.1; p as u32 + 20 } });
#[derive(Clone, Copy)] enum E302 { Live(Result<Never, Option<bool>>), U1, U4, D0([u32; 0], Void), U3, U5, U0, U2 }
suite!(302, s302, E302, [E302::Live(Err(Some(true))), E302::Live(Err(Some(false))), E302::Live(Err(None)), E302::U1, E302::U4, E302::U3, E302::U5, E302::U0, E302::U2], [E302::Live(Err(black_box(Some(true)))), E302::Live(Err(black_box(Some(false)))), E302::Live(Err(black_box(None)))], |e| match e { E302::Live(w) => { let p: Option<bool> = (match w { Err(x) => x, Ok(_) => loop {} }); match p { Some(b) => 40 + b as u32, None => 49 } }, E302::U1 => 1002, E302::U4 => 1005, E302::D0(..) => 9001, E302::U3 => 1004, E302::U5 => 1006, E302::U0 => 1001, E302::U2 => 1003 });
#[derive(Clone, Copy)] enum E303 { D0(PhantomData<Void>), Live(Wrap<Tr<((), NonZeroU16)>>) }
suite!(303, s303, E303, [E303::D0(PhantomData), E303::Live(Wrap(Tr(((), NonZeroU16::new(1).unwrap())))), E303::Live(Wrap(Tr(((), NonZeroU16::new(65535).unwrap()))))], [E303::Live(Wrap(Tr(((), black_box(NonZeroU16::new(1).unwrap()))))), E303::Live(Wrap(Tr(((), black_box(NonZeroU16::new(65535).unwrap())))))], |e| match e { E303::D0(..) => 2001, E303::Live(w) => { let p: NonZeroU16 = w.0.0.1; p.get() as u32 } });
#[derive(Clone, Copy)] enum E304 { Live((Sg3, PhantomData<Void>, [Void; 0])), D0(ManuallyDrop<Void>), U0, U1, D1(Wrap<Void>), U2 }
suite!(304, s304, E304, [E304::Live((Sg3::A, PhantomData, [])), E304::Live((Sg3::B, PhantomData, [])), E304::U0, E304::U1, E304::U2], [E304::Live((black_box(Sg3::A), PhantomData, [])), E304::Live((black_box(Sg3::B), PhantomData, []))], |e| match e { E304::Live(w) => { let p: Sg3 = w.0; p as i32 as u32 }, E304::D0(..) => 9001, E304::U0 => 1001, E304::U1 => 1002, E304::D1(..) => 9002, E304::U2 => 1003 });
#[derive(Clone, Copy)] enum E305 { U1, U0, Live(SL<Wrap<SL3<Three>>>) }
suite!(305, s305, E305, [E305::U1, E305::U0, E305::Live(SL::Live(Wrap(SL3::Live(Three::A)))), E305::Live(SL::Live(Wrap(SL3::Live(Three::B)))), E305::Live(SL::Live(Wrap(SL3::Live(Three::C))))], [E305::Live(SL::Live(Wrap(SL3::Live(black_box(Three::A))))), E305::Live(SL::Live(Wrap(SL3::Live(black_box(Three::B))))), E305::Live(SL::Live(Wrap(SL3::Live(black_box(Three::C)))))], |e| match e { E305::U1 => 1002, E305::U0 => 1001, E305::Live(w) => { let p: Three = (match w { SL::Live(Wrap(SL3::Live(x))) => x, _ => loop {} }); p as u32 + 20 } });
#[derive(Clone, Copy)] enum E306 { D1(Option<Void>), U0, D2([(Void,); 0]), Live(SL<W64>), U1, D0(&'static Void, Void) }
suite!(306, s306, E306, [E306::D1(None), E306::U0, E306::D2([]), E306::Live(SL::Live(W64::A)), E306::Live(SL::Live(W64::B)), E306::U1], [E306::Live(SL::Live(black_box(W64::A))), E306::Live(SL::Live(black_box(W64::B)))], |e| match e { E306::D1(..) => 2002, E306::U0 => 1001, E306::D2(..) => 2003, E306::Live(w) => { let p: W64 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); (p as u64 % 1_000_003) as u32 }, E306::U1 => 1002, E306::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E307 { D0(PkDead), D1([Void; 2]), Live(Wrap<NonZeroI16>), U1, D2(Void, (), PhantomData<u8>), U0, U2 }
suite!(307, s307, E307, [E307::Live(Wrap(NonZeroI16::new(-1).unwrap())), E307::Live(Wrap(NonZeroI16::new(300).unwrap())), E307::U1, E307::U0, E307::U2], [E307::Live(Wrap(black_box(NonZeroI16::new(-1).unwrap()))), E307::Live(Wrap(black_box(NonZeroI16::new(300).unwrap())))], |e| match e { E307::D0(..) => 9001, E307::D1(..) => 9002, E307::Live(w) => { let p: NonZeroI16 = w.0; p.get() as i32 as u32 }, E307::U1 => 1002, E307::D2(..) => 9003, E307::U0 => 1001, E307::U2 => 1003 });
#[derive(Clone, Copy)] enum E308 { Live(Pk<S32>) }
suite!(308, s308, E308, [E308::Live(Pk(S32::A)), E308::Live(Pk(S32::B))], [E308::Live(Pk(black_box(S32::A))), E308::Live(Pk(black_box(S32::B)))], |e| match e { E308::Live(w) => { let p: S32 = { w.0 }; p as i32 as u32 } });
#[derive(Clone, Copy)] enum E309 { U2, U1, U3, U4, U0, Live(SL<W16>), D0([u16; 0], Void), D1((Void, ())), U5 }
suite!(309, s309, E309, [E309::U2, E309::U1, E309::U3, E309::U4, E309::U0, E309::Live(SL::Live(W16::A)), E309::Live(SL::Live(W16::B)), E309::U5], [E309::Live(SL::Live(black_box(W16::A))), E309::Live(SL::Live(black_box(W16::B)))], |e| match e { E309::U2 => 1003, E309::U1 => 1002, E309::U3 => 1004, E309::U4 => 1005, E309::U0 => 1001, E309::Live(w) => { let p: W16 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); p as u32 }, E309::D0(..) => 9001, E309::D1(..) => 9002, E309::U5 => 1006 });
#[derive(Clone, Copy)] enum E310 { D1([u32; 0]), U0, D2(UDead), Live(CW<NonZeroU64>), D0(([Void; 0], Void)) }
suite!(310, s310, E310, [E310::D1([]), E310::U0, E310::Live(CW(NonZeroU64::new(1).unwrap())), E310::Live(CW(NonZeroU64::new(0xFFFF_FFFF_FFFF_FFFF).unwrap())), E310::Live(CW(NonZeroU64::new(0x1_0000_0000).unwrap()))], [E310::Live(CW(black_box(NonZeroU64::new(1).unwrap()))), E310::Live(CW(black_box(NonZeroU64::new(0xFFFF_FFFF_FFFF_FFFF).unwrap()))), E310::Live(CW(black_box(NonZeroU64::new(0x1_0000_0000).unwrap())))], |e| match e { E310::D1(..) => 2002, E310::U0 => 1001, E310::D2(..) => 9003, E310::Live(w) => { let p: NonZeroU64 = w.0; (p.get() % 1_000_003) as u32 }, E310::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E311 { D0([Void; 0]), U0, U1, Live(S64) }
suite!(311, s311, E311, [E311::D0([]), E311::U0, E311::U1, E311::Live(S64::A), E311::Live(S64::B)], [E311::Live(black_box(S64::A)), E311::Live(black_box(S64::B))], |e| match e { E311::D0(..) => 2001, E311::U0 => 1001, E311::U1 => 1002, E311::Live(w) => { let p: S64 = w; (p as i64 as u64 % 1_000_003) as u32 } });
#[derive(Clone, Copy)] enum E312 { D0(Wrap<Void>), D1(NamedDead), U0, Live(Hi2) }
suite!(312, s312, E312, [E312::U0, E312::Live(Hi2::A), E312::Live(Hi2::B)], [E312::Live(black_box(Hi2::A)), E312::Live(black_box(Hi2::B))], |e| match e { E312::D0(..) => 9001, E312::D1(..) => 9002, E312::U0 => 1001, E312::Live(w) => { let p: Hi2 = w; p as u32 } });
#[derive(Clone, Copy)] enum E313 { U1, U0, Live((fn() -> u32,)), U3, U2 }
suite!(313, s313, E313, [E313::U1, E313::U0, E313::Live((seven,)), E313::Live((eight,)), E313::U3, E313::U2], [E313::Live((black_box(seven),)), E313::Live((black_box(eight),))], |e| match e { E313::U1 => 1002, E313::U0 => 1001, E313::Live(w) => { let p: fn() -> u32 = w.0; p() }, E313::U3 => 1004, E313::U2 => 1003 });
#[derive(Clone, Copy)] enum E314 { Live(Option<bool>), D0(NamedDead), U1, U0, D1([u32; 0], Void) }
suite!(314, s314, E314, [E314::Live(Some(true)), E314::Live(Some(false)), E314::Live(None), E314::U1, E314::U0], [E314::Live(black_box(Some(true))), E314::Live(black_box(Some(false))), E314::Live(black_box(None))], |e| match e { E314::Live(w) => { let p: Option<bool> = w; match p { Some(b) => 40 + b as u32, None => 49 } }, E314::D0(..) => 9001, E314::U1 => 1002, E314::U0 => 1001, E314::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E315 { Live(Result<Never, Sg2>) }
suite!(315, s315, E315, [E315::Live(Err(Sg2::A)), E315::Live(Err(Sg2::B))], [E315::Live(Err(black_box(Sg2::A))), E315::Live(Err(black_box(Sg2::B)))], |e| match e { E315::Live(w) => { let p: Sg2 = (match w { Err(x) => x, Ok(_) => loop {} }); p as i32 as u32 } });
#[derive(Clone, Copy)] enum E316 { U0, Live(Wrap<NonZeroU8>) }
suite!(316, s316, E316, [E316::U0, E316::Live(Wrap(NonZeroU8::new(1).unwrap())), E316::Live(Wrap(NonZeroU8::new(255).unwrap()))], [E316::Live(Wrap(black_box(NonZeroU8::new(1).unwrap()))), E316::Live(Wrap(black_box(NonZeroU8::new(255).unwrap())))], |e| match e { E316::U0 => 1001, E316::Live(w) => { let p: NonZeroU8 = w.0; p.get() as u32 } });
#[derive(Clone, Copy)] enum E317 { U1, D0([Void; 2]), U0, Live((Three,)), D1([[Void; 1]; 1]) }
suite!(317, s317, E317, [E317::U1, E317::U0, E317::Live((Three::A,)), E317::Live((Three::B,)), E317::Live((Three::C,))], [E317::Live((black_box(Three::A),)), E317::Live((black_box(Three::B),)), E317::Live((black_box(Three::C),))], |e| match e { E317::U1 => 1002, E317::D0(..) => 9001, E317::U0 => 1001, E317::Live(w) => { let p: Three = w.0; p as u32 + 20 }, E317::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E318 { D0(([Void; 0], Void)), Live(Result<NonZeroI8, Void>), D1((Never, Never)), U0 }
suite!(318, s318, E318, [E318::Live(Ok(NonZeroI8::new(-1).unwrap())), E318::Live(Ok(NonZeroI8::new(127).unwrap())), E318::U0], [E318::Live(Ok(black_box(NonZeroI8::new(-1).unwrap()))), E318::Live(Ok(black_box(NonZeroI8::new(127).unwrap())))], |e| match e { E318::D0(..) => 9001, E318::Live(w) => { let p: NonZeroI8 = (match w { Ok(x) => x, Err(_) => loop {} }); p.get() as i32 as u32 }, E318::D1(..) => 9002, E318::U0 => 1001 });
#[derive(Clone, Copy)] enum E319 { Live((W32,)), D0(Wrap<Void>) }
suite!(319, s319, E319, [E319::Live((W32::A,)), E319::Live((W32::B,))], [E319::Live((black_box(W32::A),)), E319::Live((black_box(W32::B),))], |e| match e { E319::Live(w) => { let p: W32 = w.0; p as u32 }, E319::D0(..) => 9001 });

#[unsafe(no_mangle)]
pub fn rust_entry() {
    s300(); s301(); s302(); s303(); s304(); s305(); s306(); s307(); s308(); s309(); s310(); s311(); s312(); s313(); s314(); s315(); s316(); s317(); s318(); s319();
}
