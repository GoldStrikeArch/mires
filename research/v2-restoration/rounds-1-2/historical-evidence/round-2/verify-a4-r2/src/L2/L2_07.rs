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
#[derive(Clone, Copy)] enum E140 { D0(DeepDead), Live(((), Hi3)) }
suite!(140, s140, E140, [E140::Live(((), Hi3::A)), E140::Live(((), Hi3::B)), E140::Live(((), Hi3::C))], |e| match e { E140::D0(..) => 9001, E140::Live(w) => { let p: Hi3 = w.1; p as u32 } });
#[derive(Clone, Copy)] enum E141 { D0((Never, Never)), Live((char,)) }
suite!(141, s141, E141, [E141::Live(('a',)), E141::Live(('\u{10FFFF}',)), E141::Live(('\0',))], |e| match e { E141::D0(..) => 9001, E141::Live(w) => { let p: char = w.0; p as u32 } });
#[derive(Clone, Copy)] enum E142 { U3, Live(Ordering), D0(Never), D1([[Void; 1]; 1]), U5, U2, U0, U4, U1, D2(UDead) }
suite!(142, s142, E142, [E142::U3, E142::Live(Ordering::Less), E142::Live(Ordering::Equal), E142::Live(Ordering::Greater), E142::U5, E142::U2, E142::U0, E142::U4, E142::U1], |e| match e { E142::U3 => 1004, E142::Live(w) => { let p: Ordering = w; p as i32 as u32 }, E142::D0(..) => 9001, E142::D1(..) => 9002, E142::U5 => 1006, E142::U2 => 1003, E142::U0 => 1001, E142::U4 => 1005, E142::U1 => 1002, E142::D2(..) => 9003 });
#[derive(Clone, Copy)] enum E143 { Live(NonZeroU16), U5, U1, U3, U2, U4, U0 }
suite!(143, s143, E143, [E143::Live(NonZeroU16::new(1).unwrap()), E143::Live(NonZeroU16::new(65535).unwrap()), E143::U5, E143::U1, E143::U3, E143::U2, E143::U4, E143::U0], |e| match e { E143::Live(w) => { let p: NonZeroU16 = w; p.get() as u32 }, E143::U5 => 1006, E143::U1 => 1002, E143::U3 => 1004, E143::U2 => 1003, E143::U4 => 1005, E143::U0 => 1001 });
#[derive(Clone, Copy)] enum E144 { U0, Live(((), W16)), U1 }
suite!(144, s144, E144, [E144::U0, E144::Live(((), W16::A)), E144::Live(((), W16::B)), E144::U1], |e| match e { E144::U0 => 1001, E144::Live(w) => { let p: W16 = w.1; p as u32 }, E144::U1 => 1002 });
#[derive(Clone, Copy)] enum E145 { Live(Wrap<Option<Option<Three>>>) }
suite!(145, s145, E145, [E145::Live(Wrap(Some(Some(Three::C)))), E145::Live(Wrap(Some(None))), E145::Live(Wrap(None))], |e| match e { E145::Live(w) => { let p: Option<Option<Three>> = w.0; match p { Some(Some(t)) => t as u32 + 60, Some(None) => 46, None => 45 } } });
#[derive(Clone, Copy)] enum E146 { D0(Option<Void>), D1(DeepDead), Live(Tr<Option<char>>), D2([[Void; 1]; 1]), U1, U0 }
suite!(146, s146, E146, [E146::D0(None), E146::Live(Tr(Some('z'))), E146::Live(Tr(None)), E146::U1, E146::U0], |e| match e { E146::D0(..) => 2001, E146::D1(..) => 9002, E146::Live(w) => { let p: Option<char> = w.0; match p { Some(c) => c as u32, None => 48 } }, E146::D2(..) => 9003, E146::U1 => 1002, E146::U0 => 1001 });
#[derive(Clone, Copy)] enum E147 { D0([Void; 0]), U1, U0, Live(CW<NonZeroU8>), U2 }
suite!(147, s147, E147, [E147::D0([]), E147::U1, E147::U0, E147::Live(CW(NonZeroU8::new(1).unwrap())), E147::Live(CW(NonZeroU8::new(255).unwrap())), E147::U2], |e| match e { E147::D0(..) => 2001, E147::U1 => 1002, E147::U0 => 1001, E147::Live(w) => { let p: NonZeroU8 = w.0; p.get() as u32 }, E147::U2 => 1003 });
#[derive(Clone, Copy)] enum E148 { U0, D0(Void, (), PhantomData<u8>), U1, Live(ManuallyDrop<S64>) }
suite!(148, s148, E148, [E148::U0, E148::U1, E148::Live(ManuallyDrop::new(S64::A)), E148::Live(ManuallyDrop::new(S64::B))], |e| match e { E148::U0 => 1001, E148::D0(..) => 9001, E148::U1 => 1002, E148::Live(w) => { let p: S64 = ManuallyDrop::into_inner(w); (p as i64 as u64 % 1_000_003) as u32 } });
#[derive(Clone, Copy)] enum E149 { Live([WU; 1]), D1([(Void,); 0]), D0(HalfDead), U0 }
suite!(149, s149, E149, [E149::Live([WU::A]), E149::Live([WU::B]), E149::D1([]), E149::D0(HalfDead::B), E149::U0], |e| match e { E149::Live(w) => { let p: WU = w[0]; p as usize as u32 }, E149::D1(..) => 2002, E149::D0(..) => 2001, E149::U0 => 1001 });
#[derive(Clone, Copy)] enum E150 { D0(Aligned), Live(Wrap<Three>), D1(Wrap<HalfDead>, ()) }
suite!(150, s150, E150, [E150::Live(Wrap(Three::A)), E150::Live(Wrap(Three::B)), E150::Live(Wrap(Three::C)), E150::D1(Wrap(HalfDead::B), ())], |e| match e { E150::D0(..) => 9001, E150::Live(w) => { let p: Three = w.0; p as u32 + 20 }, E150::D1(..) => 2002 });
#[derive(Clone, Copy)] enum E151 { Live(W64), U5, D0((u32, Void)), U2, U4, U0, U3, U1 }
suite!(151, s151, E151, [E151::Live(W64::A), E151::Live(W64::B), E151::U5, E151::U2, E151::U4, E151::U0, E151::U3, E151::U1], |e| match e { E151::Live(w) => { let p: W64 = w; (p as u64 % 1_000_003) as u32 }, E151::U5 => 1006, E151::D0(..) => 9001, E151::U2 => 1003, E151::U4 => 1005, E151::U0 => 1001, E151::U3 => 1004, E151::U1 => 1002 });
#[derive(Clone, Copy)] enum E152 { Live((NonZeroI8,)), U0 }
suite!(152, s152, E152, [E152::Live((NonZeroI8::new(-1).unwrap(),)), E152::Live((NonZeroI8::new(127).unwrap(),)), E152::U0], |e| match e { E152::Live(w) => { let p: NonZeroI8 = w.0; p.get() as i32 as u32 }, E152::U0 => 1001 });
#[derive(Clone, Copy)] enum E153 { D1(([Void; 0], Void)), D0(HalfDead2), Live([NonZeroI8; 1]) }
suite!(153, s153, E153, [E153::D0(HalfDead2::A), E153::Live([NonZeroI8::new(-1).unwrap()]), E153::Live([NonZeroI8::new(127).unwrap()])], |e| match e { E153::D1(..) => 9002, E153::D0(..) => 2001, E153::Live(w) => { let p: NonZeroI8 = w[0]; p.get() as i32 as u32 } });
#[derive(Clone, Copy)] enum E154 { U2, U1, D0(Void, [u64; 0]), U0, Live((NonZeroU64,)) }
suite!(154, s154, E154, [E154::U2, E154::U1, E154::U0, E154::Live((NonZeroU64::new(1).unwrap(),)), E154::Live((NonZeroU64::new(0xFFFF_FFFF_FFFF_FFFF).unwrap(),)), E154::Live((NonZeroU64::new(0x1_0000_0000).unwrap(),))], |e| match e { E154::U2 => 1003, E154::U1 => 1002, E154::D0(..) => 9001, E154::U0 => 1001, E154::Live(w) => { let p: NonZeroU64 = w.0; (p.get() % 1_000_003) as u32 } });
#[derive(Clone, Copy)] enum E155 { Live(Tr<Sg>), D2(Void), D0((), Infallible), D1((HalfDead, Void)) }
suite!(155, s155, E155, [E155::Live(Tr(Sg::A)), E155::Live(Tr(Sg::B))], |e| match e { E155::Live(w) => { let p: Sg = w.0; p as i32 as u32 }, E155::D2(..) => 9003, E155::D0(..) => 9001, E155::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E156 { Live(Wrap<char>), D0(Void) }
suite!(156, s156, E156, [E156::Live(Wrap('a')), E156::Live(Wrap('\u{10FFFF}')), E156::Live(Wrap('\0'))], |e| match e { E156::Live(w) => { let p: char = w.0; p as u32 }, E156::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E157 { Live([NonZeroU32; 1]), U1, D0(TrDead), U0 }
suite!(157, s157, E157, [E157::Live([NonZeroU32::new(1).unwrap()]), E157::Live([NonZeroU32::new(0xFFFF_FFFF).unwrap()]), E157::U1, E157::U0], |e| match e { E157::Live(w) => { let p: NonZeroU32 = w[0]; p.get() }, E157::U1 => 1002, E157::D0(..) => 9001, E157::U0 => 1001 });
#[derive(Clone, Copy)] enum E158 { Live(Named<S32>), U1, U0, U5, U4, U2, D0([(Void,); 3]), U3 }
suite!(158, s158, E158, [E158::Live(Named { z: (), v: S32::A, p: PhantomData }), E158::Live(Named { z: (), v: S32::B, p: PhantomData }), E158::U1, E158::U0, E158::U5, E158::U4, E158::U2, E158::U3], |e| match e { E158::Live(w) => { let p: S32 = w.v; p as i32 as u32 }, E158::U1 => 1002, E158::U0 => 1001, E158::U5 => 1006, E158::U4 => 1005, E158::U2 => 1003, E158::D0(..) => 9001, E158::U3 => 1004 });
#[derive(Clone, Copy)] enum E159 { Live(SL<NonZeroI16>), U0, U1, D1(HalfDead), D0([Void; 2]) }
suite!(159, s159, E159, [E159::Live(SL::Live(NonZeroI16::new(-1).unwrap())), E159::Live(SL::Live(NonZeroI16::new(300).unwrap())), E159::U0, E159::U1, E159::D1(HalfDead::B)], |e| match e { E159::Live(w) => { let p: NonZeroI16 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); p.get() as i32 as u32 }, E159::U0 => 1001, E159::U1 => 1002, E159::D1(..) => 2002, E159::D0(..) => 9001 });

#[unsafe(no_mangle)]
pub fn rust_entry() {
    s140(); s141(); s142(); s143(); s144(); s145(); s146(); s147(); s148(); s149(); s150(); s151(); s152(); s153(); s154(); s155(); s156(); s157(); s158(); s159();
}
