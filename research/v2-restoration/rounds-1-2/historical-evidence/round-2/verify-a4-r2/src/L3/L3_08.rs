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
#[derive(Clone, Copy)] enum E160 { Live(((), char)), D1([u32; 0]), U0, D0((Void, ())), U1 }
suite!(160, s160, E160, [E160::Live(((), 'a')), E160::Live(((), '\u{10FFFF}')), E160::Live(((), '\0')), E160::D1([]), E160::U0, E160::U1], [E160::Live(((), black_box('a'))), E160::Live(((), black_box('\u{10FFFF}'))), E160::Live(((), black_box('\0')))], |e| match e { E160::Live(w) => { let p: char = w.1; p as u32 }, E160::D1(..) => 2002, E160::U0 => 1001, E160::D0(..) => 9001, E160::U1 => 1002 });
#[derive(Clone, Copy)] enum E161 { U0, D0((Never, Never)), Live(Result<S32, Void>) }
suite!(161, s161, E161, [E161::U0, E161::Live(Ok(S32::A)), E161::Live(Ok(S32::B))], [E161::Live(Ok(black_box(S32::A))), E161::Live(Ok(black_box(S32::B)))], |e| match e { E161::U0 => 1001, E161::D0(..) => 9001, E161::Live(w) => { let p: S32 = (match w { Ok(x) => x, Err(_) => loop {} }); p as i32 as u32 } });
#[derive(Clone, Copy)] enum E162 { D0(AllDead), Live(SL<Wrap<SL3<NonZeroU8>>>) }
suite!(162, s162, E162, [E162::Live(SL::Live(Wrap(SL3::Live(NonZeroU8::new(1).unwrap())))), E162::Live(SL::Live(Wrap(SL3::Live(NonZeroU8::new(255).unwrap()))))], [E162::Live(SL::Live(Wrap(SL3::Live(black_box(NonZeroU8::new(1).unwrap()))))), E162::Live(SL::Live(Wrap(SL3::Live(black_box(NonZeroU8::new(255).unwrap())))))], |e| match e { E162::D0(..) => 9001, E162::Live(w) => { let p: NonZeroU8 = (match w { SL::Live(Wrap(SL3::Live(x))) => x, _ => loop {} }); p.get() as u32 } });
#[derive(Clone, Copy)] enum E163 { U1, U0, D0(HalfDead), Live(SL<Hi2>) }
suite!(163, s163, E163, [E163::U1, E163::U0, E163::D0(HalfDead::B), E163::Live(SL::Live(Hi2::A)), E163::Live(SL::Live(Hi2::B))], [E163::Live(SL::Live(black_box(Hi2::A))), E163::Live(SL::Live(black_box(Hi2::B)))], |e| match e { E163::U1 => 1002, E163::U0 => 1001, E163::D0(..) => 2001, E163::Live(w) => { let p: Hi2 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); p as u32 } });
#[derive(Clone, Copy)] enum E164 { U0, Live((Sg3, PhantomData<Void>, [Void; 0])), D1(PhantomData<Void>), D0(Result<Void, Never>) }
suite!(164, s164, E164, [E164::U0, E164::Live((Sg3::A, PhantomData, [])), E164::Live((Sg3::B, PhantomData, [])), E164::D1(PhantomData)], [E164::Live((black_box(Sg3::A), PhantomData, [])), E164::Live((black_box(Sg3::B), PhantomData, []))], |e| match e { E164::U0 => 1001, E164::Live(w) => { let p: Sg3 = w.0; p as i32 as u32 }, E164::D1(..) => 2002, E164::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E165 { D0((), Infallible), Live(Result<NonZeroU64, Void>), U0 }
suite!(165, s165, E165, [E165::Live(Ok(NonZeroU64::new(1).unwrap())), E165::Live(Ok(NonZeroU64::new(0xFFFF_FFFF_FFFF_FFFF).unwrap())), E165::Live(Ok(NonZeroU64::new(0x1_0000_0000).unwrap())), E165::U0], [E165::Live(Ok(black_box(NonZeroU64::new(1).unwrap()))), E165::Live(Ok(black_box(NonZeroU64::new(0xFFFF_FFFF_FFFF_FFFF).unwrap()))), E165::Live(Ok(black_box(NonZeroU64::new(0x1_0000_0000).unwrap())))], |e| match e { E165::D0(..) => 9001, E165::Live(w) => { let p: NonZeroU64 = (match w { Ok(x) => x, Err(_) => loop {} }); (p.get() % 1_000_003) as u32 }, E165::U0 => 1001 });
#[derive(Clone, Copy)] enum E166 { D1(Result<Void, Never>), D0(Void), Live(CW<fn() -> u32>) }
suite!(166, s166, E166, [E166::Live(CW(seven)), E166::Live(CW(eight))], [E166::Live(CW(black_box(seven))), E166::Live(CW(black_box(eight)))], |e| match e { E166::D1(..) => 9002, E166::D0(..) => 9001, E166::Live(w) => { let p: fn() -> u32 = w.0; p() } });
#[derive(Clone, Copy)] enum E167 { Live(CW<W16>), U3, U1, D0(Result<(), Void>), U2, U0 }
suite!(167, s167, E167, [E167::Live(CW(W16::A)), E167::Live(CW(W16::B)), E167::U3, E167::U1, E167::D0(Ok(())), E167::U2, E167::U0], [E167::Live(CW(black_box(W16::A))), E167::Live(CW(black_box(W16::B)))], |e| match e { E167::Live(w) => { let p: W16 = w.0; p as u32 }, E167::U3 => 1004, E167::U1 => 1002, E167::D0(..) => 2001, E167::U2 => 1003, E167::U0 => 1001 });
#[derive(Clone, Copy)] enum E168 { U1, U0, U2, D0(Wrap<Wrap<(Void, ())>>), Live(((), S64)) }
suite!(168, s168, E168, [E168::U1, E168::U0, E168::U2, E168::Live(((), S64::A)), E168::Live(((), S64::B))], [E168::Live(((), black_box(S64::A))), E168::Live(((), black_box(S64::B)))], |e| match e { E168::U1 => 1002, E168::U0 => 1001, E168::U2 => 1003, E168::D0(..) => 9001, E168::Live(w) => { let p: S64 = w.1; (p as i64 as u64 % 1_000_003) as u32 } });
#[derive(Clone, Copy)] enum E169 { D1([u32; 0]), U0, U1, Live(NonZeroU8), D0(Wrap<AllDead>) }
suite!(169, s169, E169, [E169::D1([]), E169::U0, E169::U1, E169::Live(NonZeroU8::new(1).unwrap()), E169::Live(NonZeroU8::new(255).unwrap())], [E169::Live(black_box(NonZeroU8::new(1).unwrap())), E169::Live(black_box(NonZeroU8::new(255).unwrap()))], |e| match e { E169::D1(..) => 2002, E169::U0 => 1001, E169::U1 => 1002, E169::Live(w) => { let p: NonZeroU8 = w; p.get() as u32 }, E169::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E170 { D0(Wrap<Void>), U0, Live(Wrap<Tr<((), Option<char>)>>) }
suite!(170, s170, E170, [E170::U0, E170::Live(Wrap(Tr(((), Some('z'))))), E170::Live(Wrap(Tr(((), None))))], [E170::Live(Wrap(Tr(((), black_box(Some('z')))))), E170::Live(Wrap(Tr(((), black_box(None)))))], |e| match e { E170::D0(..) => 9001, E170::U0 => 1001, E170::Live(w) => { let p: Option<char> = w.0.0.1; match p { Some(c) => c as u32, None => 48 } } });
#[derive(Clone, Copy)] enum E171 { D0(Option<Void>), Live(Result<Never, bool>), D1(Wrap<Void>) }
suite!(171, s171, E171, [E171::D0(None), E171::Live(Err(true)), E171::Live(Err(false))], [E171::Live(Err(black_box(true))), E171::Live(Err(black_box(false)))], |e| match e { E171::D0(..) => 2001, E171::Live(w) => { let p: bool = (match w { Err(x) => x, Ok(_) => loop {} }); 10 + p as u32 }, E171::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E172 { U0, Live(Named<NonZeroI16>), U3, D1(Aligned), D0(Option<Void>), U1, U2 }
suite!(172, s172, E172, [E172::U0, E172::Live(Named { z: (), v: NonZeroI16::new(-1).unwrap(), p: PhantomData }), E172::Live(Named { z: (), v: NonZeroI16::new(300).unwrap(), p: PhantomData }), E172::U3, E172::D0(None), E172::U1, E172::U2], [E172::Live(Named { z: (), v: black_box(NonZeroI16::new(-1).unwrap()), p: PhantomData }), E172::Live(Named { z: (), v: black_box(NonZeroI16::new(300).unwrap()), p: PhantomData })], |e| match e { E172::U0 => 1001, E172::Live(w) => { let p: NonZeroI16 = w.v; p.get() as i32 as u32 }, E172::U3 => 1004, E172::D1(..) => 9002, E172::D0(..) => 2001, E172::U1 => 1002, E172::U2 => 1003 });
#[derive(Clone, Copy)] enum E173 { Live((Hi,)), U0, D1(Wrap<Wrap<(Void, ())>>), D0(Result<(), Void>) }
suite!(173, s173, E173, [E173::Live((Hi::A,)), E173::Live((Hi::B,)), E173::U0, E173::D0(Ok(()))], [E173::Live((black_box(Hi::A),)), E173::Live((black_box(Hi::B),))], |e| match e { E173::Live(w) => { let p: Hi = w.0; p as u32 }, E173::U0 => 1001, E173::D1(..) => 9002, E173::D0(..) => 2001 });
#[derive(Clone, Copy)] enum E174 { D1(ManuallyDrop<Void>), U0, D0(UDead), Live(Result<Never, Option<bool>>) }
suite!(174, s174, E174, [E174::U0, E174::Live(Err(Some(true))), E174::Live(Err(Some(false))), E174::Live(Err(None))], [E174::Live(Err(black_box(Some(true)))), E174::Live(Err(black_box(Some(false)))), E174::Live(Err(black_box(None)))], |e| match e { E174::D1(..) => 9002, E174::U0 => 1001, E174::D0(..) => 9001, E174::Live(w) => { let p: Option<bool> = (match w { Err(x) => x, Ok(_) => loop {} }); match p { Some(b) => 40 + b as u32, None => 49 } } });
#[derive(Clone, Copy)] enum E175 { D0((), Infallible), U0, Live(Result<NonZeroU32, Void>) }
suite!(175, s175, E175, [E175::U0, E175::Live(Ok(NonZeroU32::new(1).unwrap())), E175::Live(Ok(NonZeroU32::new(0xFFFF_FFFF).unwrap()))], [E175::Live(Ok(black_box(NonZeroU32::new(1).unwrap()))), E175::Live(Ok(black_box(NonZeroU32::new(0xFFFF_FFFF).unwrap())))], |e| match e { E175::D0(..) => 9001, E175::U0 => 1001, E175::Live(w) => { let p: NonZeroU32 = (match w { Ok(x) => x, Err(_) => loop {} }); p.get() } });
#[derive(Clone, Copy)] enum E176 { D0([Void; 0]), U2, Live(Result<Never, Option<Hi2>>), U0, U1 }
suite!(176, s176, E176, [E176::D0([]), E176::U2, E176::Live(Err(Some(Hi2::A))), E176::Live(Err(Some(Hi2::B))), E176::Live(Err(None)), E176::U0, E176::U1], [E176::Live(Err(black_box(Some(Hi2::A)))), E176::Live(Err(black_box(Some(Hi2::B)))), E176::Live(Err(black_box(None)))], |e| match e { E176::D0(..) => 2001, E176::U2 => 1003, E176::Live(w) => { let p: Option<Hi2> = (match w { Err(x) => x, Ok(_) => loop {} }); match p { Some(h) => h as u32, None => 47 } }, E176::U0 => 1001, E176::U1 => 1002 });
#[derive(Clone, Copy)] enum E177 { Live([NonZeroU64; 1]), D0((Never, Never)) }
suite!(177, s177, E177, [E177::Live([NonZeroU64::new(1).unwrap()]), E177::Live([NonZeroU64::new(0xFFFF_FFFF_FFFF_FFFF).unwrap()]), E177::Live([NonZeroU64::new(0x1_0000_0000).unwrap()])], [E177::Live([black_box(NonZeroU64::new(1).unwrap())]), E177::Live([black_box(NonZeroU64::new(0xFFFF_FFFF_FFFF_FFFF).unwrap())]), E177::Live([black_box(NonZeroU64::new(0x1_0000_0000).unwrap())])], |e| match e { E177::Live(w) => { let p: NonZeroU64 = w[0]; (p.get() % 1_000_003) as u32 }, E177::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E178 { Live(Named<Sg>), D0(TrDead), U0 }
suite!(178, s178, E178, [E178::Live(Named { z: (), v: Sg::A, p: PhantomData }), E178::Live(Named { z: (), v: Sg::B, p: PhantomData }), E178::U0], [E178::Live(Named { z: (), v: black_box(Sg::A), p: PhantomData }), E178::Live(Named { z: (), v: black_box(Sg::B), p: PhantomData })], |e| match e { E178::Live(w) => { let p: Sg = w.v; p as i32 as u32 }, E178::D0(..) => 9001, E178::U0 => 1001 });
#[derive(Clone, Copy)] enum E179 { D0(ManuallyDrop<Void>), Live((bool,)) }
suite!(179, s179, E179, [E179::Live((true,)), E179::Live((false,))], [E179::Live((black_box(true),)), E179::Live((black_box(false),))], |e| match e { E179::D0(..) => 9001, E179::Live(w) => { let p: bool = w.0; 10 + p as u32 } });

#[unsafe(no_mangle)]
pub fn rust_entry() {
    s160(); s161(); s162(); s163(); s164(); s165(); s166(); s167(); s168(); s169(); s170(); s171(); s172(); s173(); s174(); s175(); s176(); s177(); s178(); s179();
}
