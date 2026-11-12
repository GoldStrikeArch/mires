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
#[derive(Clone, Copy)] enum E160 { U0, Live(S32), U1, D0(PhantomData<Void>), D1(HalfDead) }
suite!(160, s160, E160, [E160::U0, E160::Live(S32::A), E160::Live(S32::B), E160::U1, E160::D0(PhantomData), E160::D1(HalfDead::B)], |e| match e { E160::U0 => 1001, E160::Live(w) => { let p: S32 = w; p as i32 as u32 }, E160::U1 => 1002, E160::D0(..) => 2001, E160::D1(..) => 2002 });
#[derive(Clone, Copy)] enum E161 { U1, Live(Result<Never, W16>), D0(DeepDead), U0 }
suite!(161, s161, E161, [E161::U1, E161::Live(Err(W16::A)), E161::Live(Err(W16::B)), E161::U0], |e| match e { E161::U1 => 1002, E161::Live(w) => { let p: W16 = (match w { Err(x) => x, Ok(_) => loop {} }); p as u32 }, E161::D0(..) => 9001, E161::U0 => 1001 });
#[derive(Clone, Copy)] enum E162 { Live(SL<NonZeroU32>), D1(Void, [u64; 0]), D0([Void; 2]), U0 }
suite!(162, s162, E162, [E162::Live(SL::Live(NonZeroU32::new(1).unwrap())), E162::Live(SL::Live(NonZeroU32::new(0xFFFF_FFFF).unwrap())), E162::U0], |e| match e { E162::Live(w) => { let p: NonZeroU32 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); p.get() }, E162::D1(..) => 9002, E162::D0(..) => 9001, E162::U0 => 1001 });
#[derive(Clone, Copy)] enum E163 { D1(u8, Void), U0, D0(DeepDead), Live(SL3<Hi>) }
suite!(163, s163, E163, [E163::U0, E163::Live(SL3::Live(Hi::A)), E163::Live(SL3::Live(Hi::B))], |e| match e { E163::D1(..) => 9002, E163::U0 => 1001, E163::D0(..) => 9001, E163::Live(w) => { let p: Hi = (match w { SL3::Live(x) => x, _ => loop {} }); p as u32 } });
#[derive(Clone, Copy)] enum E164 { D0([(Void,); 3]), Live(ManuallyDrop<Option<char>>), D1(HalfDead) }
suite!(164, s164, E164, [E164::Live(ManuallyDrop::new(Some('z'))), E164::Live(ManuallyDrop::new(None)), E164::D1(HalfDead::B)], |e| match e { E164::D0(..) => 9001, E164::Live(w) => { let p: Option<char> = ManuallyDrop::into_inner(w); match p { Some(c) => c as u32, None => 48 } }, E164::D1(..) => 2002 });
#[derive(Clone, Copy)] enum E165 { Live(((), Ordering)), U0, D0(DeepDead) }
suite!(165, s165, E165, [E165::Live(((), Ordering::Less)), E165::Live(((), Ordering::Equal)), E165::Live(((), Ordering::Greater)), E165::U0], |e| match e { E165::Live(w) => { let p: Ordering = w.1; p as i32 as u32 }, E165::U0 => 1001, E165::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E166 { Live(Pk<Option<char>>), U0, D1(MaybeUninit<Void>), D0(TrDead) }
suite!(166, s166, E166, [E166::Live(Pk(Some('z'))), E166::Live(Pk(None)), E166::U0], |e| match e { E166::Live(w) => { let p: Option<char> = { w.0 }; match p { Some(c) => c as u32, None => 48 } }, E166::U0 => 1001, E166::D1(..) => 9002, E166::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E167 { U1, U2, U0, D0(HalfDead2), Live(NonZeroI16), D1(Wrap<Wrap<(Void, ())>>) }
suite!(167, s167, E167, [E167::U1, E167::U2, E167::U0, E167::D0(HalfDead2::A), E167::Live(NonZeroI16::new(-1).unwrap()), E167::Live(NonZeroI16::new(300).unwrap())], |e| match e { E167::U1 => 1002, E167::U2 => 1003, E167::U0 => 1001, E167::D0(..) => 2001, E167::Live(w) => { let p: NonZeroI16 = w; p.get() as i32 as u32 }, E167::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E168 { D1([u16; 0], Void), Live(((), Option<Option<Three>>)), D0((Void,)) }
suite!(168, s168, E168, [E168::Live(((), Some(Some(Three::C)))), E168::Live(((), Some(None))), E168::Live(((), None))], |e| match e { E168::D1(..) => 9002, E168::Live(w) => { let p: Option<Option<Three>> = w.1; match p { Some(Some(t)) => t as u32 + 60, Some(None) => 46, None => 45 } }, E168::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E169 { D2(Void), U0, Live((NonZeroU32, PhantomData<Void>, [Void; 0])), D1(Wrap<AllDead>), U1, D0([u32; 0]) }
suite!(169, s169, E169, [E169::U0, E169::Live((NonZeroU32::new(1).unwrap(), PhantomData, [])), E169::Live((NonZeroU32::new(0xFFFF_FFFF).unwrap(), PhantomData, [])), E169::U1, E169::D0([])], |e| match e { E169::D2(..) => 9003, E169::U0 => 1001, E169::Live(w) => { let p: NonZeroU32 = w.0; p.get() }, E169::D1(..) => 9002, E169::U1 => 1002, E169::D0(..) => 2001 });
#[derive(Clone, Copy)] enum E170 { Live(CW<NonZeroU64>), D0([Void; 2]), U1, D2(Wrap<HalfDead>, ()), U0, U2, D1(u8, Void) }
suite!(170, s170, E170, [E170::Live(CW(NonZeroU64::new(1).unwrap())), E170::Live(CW(NonZeroU64::new(0xFFFF_FFFF_FFFF_FFFF).unwrap())), E170::Live(CW(NonZeroU64::new(0x1_0000_0000).unwrap())), E170::U1, E170::D2(Wrap(HalfDead::B), ()), E170::U0, E170::U2], |e| match e { E170::Live(w) => { let p: NonZeroU64 = w.0; (p.get() % 1_000_003) as u32 }, E170::D0(..) => 9001, E170::U1 => 1002, E170::D2(..) => 2003, E170::U0 => 1001, E170::U2 => 1003, E170::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E171 { U2, U1, Live(Ordering), U0 }
suite!(171, s171, E171, [E171::U2, E171::U1, E171::Live(Ordering::Less), E171::Live(Ordering::Equal), E171::Live(Ordering::Greater), E171::U0], |e| match e { E171::U2 => 1003, E171::U1 => 1002, E171::Live(w) => { let p: Ordering = w; p as i32 as u32 }, E171::U0 => 1001 });
#[derive(Clone, Copy)] enum E172 { U2, U1, U3, U0, D0([u32; 0], Void), D1(Never), Live(Pk<Hi3>), U4, U5 }
suite!(172, s172, E172, [E172::U2, E172::U1, E172::U3, E172::U0, E172::Live(Pk(Hi3::A)), E172::Live(Pk(Hi3::B)), E172::Live(Pk(Hi3::C)), E172::U4, E172::U5], |e| match e { E172::U2 => 1003, E172::U1 => 1002, E172::U3 => 1004, E172::U0 => 1001, E172::D0(..) => 9001, E172::D1(..) => 9002, E172::Live(w) => { let p: Hi3 = { w.0 }; p as u32 }, E172::U4 => 1005, E172::U5 => 1006 });
#[derive(Clone, Copy)] enum E173 { Live(ZPad<Hi>) }
suite!(173, s173, E173, [E173::Live(ZPad([], Hi::A, ())), E173::Live(ZPad([], Hi::B, ()))], |e| match e { E173::Live(w) => { let p: Hi = w.1; p as u32 } });
#[derive(Clone, Copy)] enum E174 { Live(Wrap<Tr<((), S64)>>), D0((Void, ())) }
suite!(174, s174, E174, [E174::Live(Wrap(Tr(((), S64::A)))), E174::Live(Wrap(Tr(((), S64::B))))], |e| match e { E174::Live(w) => { let p: S64 = w.0.0.1; (p as i64 as u64 % 1_000_003) as u32 }, E174::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E175 { U2, U1, Live(CW<Sg>), U0 }
suite!(175, s175, E175, [E175::U2, E175::U1, E175::Live(CW(Sg::A)), E175::Live(CW(Sg::B)), E175::U0], |e| match e { E175::U2 => 1003, E175::U1 => 1002, E175::Live(w) => { let p: Sg = w.0; p as i32 as u32 }, E175::U0 => 1001 });
#[derive(Clone, Copy)] enum E176 { U0, U2, D1(Result<(), Void>), U1, D0(NamedDead), Live(SL<Wrap<SL3<Option<char>>>>) }
suite!(176, s176, E176, [E176::U0, E176::U2, E176::D1(Ok(())), E176::U1, E176::Live(SL::Live(Wrap(SL3::Live(Some('z'))))), E176::Live(SL::Live(Wrap(SL3::Live(None))))], |e| match e { E176::U0 => 1001, E176::U2 => 1003, E176::D1(..) => 2002, E176::U1 => 1002, E176::D0(..) => 9001, E176::Live(w) => { let p: Option<char> = (match w { SL::Live(Wrap(SL3::Live(x))) => x, _ => loop {} }); match p { Some(c) => c as u32, None => 48 } } });
#[derive(Clone, Copy)] enum E177 { D2(Result<(), Void>), D0(Aligned), D1([u32; 0]), Live(Pk<bool>), U0 }
suite!(177, s177, E177, [E177::D2(Ok(())), E177::D1([]), E177::Live(Pk(true)), E177::Live(Pk(false)), E177::U0], |e| match e { E177::D2(..) => 2003, E177::D0(..) => 9001, E177::D1(..) => 2002, E177::Live(w) => { let p: bool = { w.0 }; 10 + p as u32 }, E177::U0 => 1001 });
#[derive(Clone, Copy)] enum E178 { D0((Void, ())), D2((Never, Never)), U1, D1([Void; 0]), U0, Live(((), NonZeroU64)) }
suite!(178, s178, E178, [E178::U1, E178::D1([]), E178::U0, E178::Live(((), NonZeroU64::new(1).unwrap())), E178::Live(((), NonZeroU64::new(0xFFFF_FFFF_FFFF_FFFF).unwrap())), E178::Live(((), NonZeroU64::new(0x1_0000_0000).unwrap()))], |e| match e { E178::D0(..) => 9001, E178::D2(..) => 9003, E178::U1 => 1002, E178::D1(..) => 2002, E178::U0 => 1001, E178::Live(w) => { let p: NonZeroU64 = w.1; (p.get() % 1_000_003) as u32 } });
#[derive(Clone, Copy)] enum E179 { Live(CW<Three>), D0(Void), U0 }
suite!(179, s179, E179, [E179::Live(CW(Three::A)), E179::Live(CW(Three::B)), E179::Live(CW(Three::C)), E179::U0], |e| match e { E179::Live(w) => { let p: Three = w.0; p as u32 + 20 }, E179::D0(..) => 9001, E179::U0 => 1001 });

#[unsafe(no_mangle)]
pub fn rust_entry() {
    s160(); s161(); s162(); s163(); s164(); s165(); s166(); s167(); s168(); s169(); s170(); s171(); s172(); s173(); s174(); s175(); s176(); s177(); s178(); s179();
}
