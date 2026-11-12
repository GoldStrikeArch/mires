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
#[derive(Clone, Copy)] enum E60 { D0((Void, ())), U3, D1(([Void; 0], Void)), U2, U1, U0, Live((Option<Option<Three>>, PhantomData<Void>, [Void; 0])) }
suite!(60, s60, E60, [E60::U3, E60::U2, E60::U1, E60::U0, E60::Live((Some(Some(Three::C)), PhantomData, [])), E60::Live((Some(None), PhantomData, [])), E60::Live((None, PhantomData, []))], |e| match e { E60::D0(..) => 9001, E60::U3 => 1004, E60::D1(..) => 9002, E60::U2 => 1003, E60::U1 => 1002, E60::U0 => 1001, E60::Live(w) => { let p: Option<Option<Three>> = w.0; match p { Some(Some(t)) => t as u32 + 60, Some(None) => 46, None => 45 } } });
#[derive(Clone, Copy)] enum E61 { D1(NamedDead), D0(MaybeUninit<Void>), Live((NonZeroI16, PhantomData<Void>, [Void; 0])), D2(Result<Void, Never>) }
suite!(61, s61, E61, [E61::Live((NonZeroI16::new(-1).unwrap(), PhantomData, [])), E61::Live((NonZeroI16::new(300).unwrap(), PhantomData, []))], |e| match e { E61::D1(..) => 9002, E61::D0(..) => 9001, E61::Live(w) => { let p: NonZeroI16 = w.0; p.get() as i32 as u32 }, E61::D2(..) => 9003 });
#[derive(Clone, Copy)] enum E62 { U3, D0(AllDead), Live(((), W32)), U2, U0, D1([[Void; 1]; 1]), D2(TrDead), U1 }
suite!(62, s62, E62, [E62::U3, E62::Live(((), W32::A)), E62::Live(((), W32::B)), E62::U2, E62::U0, E62::U1], |e| match e { E62::U3 => 1004, E62::D0(..) => 9001, E62::Live(w) => { let p: W32 = w.1; p as u32 }, E62::U2 => 1003, E62::U0 => 1001, E62::D1(..) => 9002, E62::D2(..) => 9003, E62::U1 => 1002 });
#[derive(Clone, Copy)] enum E63 { U1, D0(NamedDead), U0, Live(SL<NonZeroU16>) }
suite!(63, s63, E63, [E63::U1, E63::U0, E63::Live(SL::Live(NonZeroU16::new(1).unwrap())), E63::Live(SL::Live(NonZeroU16::new(65535).unwrap()))], |e| match e { E63::U1 => 1002, E63::D0(..) => 9001, E63::U0 => 1001, E63::Live(w) => { let p: NonZeroU16 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); p.get() as u32 } });
#[derive(Clone, Copy)] enum E64 { Live(Named<Three>), D0([u32; 0]) }
suite!(64, s64, E64, [E64::Live(Named { z: (), v: Three::A, p: PhantomData }), E64::Live(Named { z: (), v: Three::B, p: PhantomData }), E64::Live(Named { z: (), v: Three::C, p: PhantomData }), E64::D0([])], |e| match e { E64::Live(w) => { let p: Three = w.v; p as u32 + 20 }, E64::D0(..) => 2001 });
#[derive(Clone, Copy)] enum E65 { D0([Void; 2]), Live(SL<Wrap<SL3<Ordering>>>) }
suite!(65, s65, E65, [E65::Live(SL::Live(Wrap(SL3::Live(Ordering::Less)))), E65::Live(SL::Live(Wrap(SL3::Live(Ordering::Equal)))), E65::Live(SL::Live(Wrap(SL3::Live(Ordering::Greater))))], |e| match e { E65::D0(..) => 9001, E65::Live(w) => { let p: Ordering = (match w { SL::Live(Wrap(SL3::Live(x))) => x, _ => loop {} }); p as i32 as u32 } });
#[derive(Clone, Copy)] enum E66 { U0, Live(SL3<Sg2>), D0([[Void; 1]; 1]), U1 }
suite!(66, s66, E66, [E66::U0, E66::Live(SL3::Live(Sg2::A)), E66::Live(SL3::Live(Sg2::B)), E66::U1], |e| match e { E66::U0 => 1001, E66::Live(w) => { let p: Sg2 = (match w { SL3::Live(x) => x, _ => loop {} }); p as i32 as u32 }, E66::D0(..) => 9001, E66::U1 => 1002 });
#[derive(Clone, Copy)] enum E67 { D0([(Void,); 0]), U1, U0, U2, Live(SL<Wrap<SL3<S64>>>) }
suite!(67, s67, E67, [E67::D0([]), E67::U1, E67::U0, E67::U2, E67::Live(SL::Live(Wrap(SL3::Live(S64::A)))), E67::Live(SL::Live(Wrap(SL3::Live(S64::B))))], |e| match e { E67::D0(..) => 2001, E67::U1 => 1002, E67::U0 => 1001, E67::U2 => 1003, E67::Live(w) => { let p: S64 = (match w { SL::Live(Wrap(SL3::Live(x))) => x, _ => loop {} }); (p as i64 as u64 % 1_000_003) as u32 } });
#[derive(Clone, Copy)] enum E68 { Live((Option<bool>,)), U1, U3, D1(()), U5, U4, U2, D0([u16; 0], Void), U0 }
suite!(68, s68, E68, [E68::Live((Some(true),)), E68::Live((Some(false),)), E68::Live((None,)), E68::U1, E68::U3, E68::D1(()), E68::U5, E68::U4, E68::U2, E68::U0], |e| match e { E68::Live(w) => { let p: Option<bool> = w.0; match p { Some(b) => 40 + b as u32, None => 49 } }, E68::U1 => 1002, E68::U3 => 1004, E68::D1(..) => 2002, E68::U5 => 1006, E68::U4 => 1005, E68::U2 => 1003, E68::D0(..) => 9001, E68::U0 => 1001 });
#[derive(Clone, Copy)] enum E69 { U1, U2, D0(ManuallyDrop<Void>), D1((u32, Void)), U3, Live(Wrap<bool>), U0 }
suite!(69, s69, E69, [E69::U1, E69::U2, E69::U3, E69::Live(Wrap(true)), E69::Live(Wrap(false)), E69::U0], |e| match e { E69::U1 => 1002, E69::U2 => 1003, E69::D0(..) => 9001, E69::D1(..) => 9002, E69::U3 => 1004, E69::Live(w) => { let p: bool = w.0; 10 + p as u32 }, E69::U0 => 1001 });
#[derive(Clone, Copy)] enum E70 { Live(&'static u32), U0, U1 }
suite!(70, s70, E70, [E70::Live(&5), E70::Live(&SEVEN), E70::U0, E70::U1], |e| match e { E70::Live(w) => { let p: &'static u32 = w; *p }, E70::U0 => 1001, E70::U1 => 1002 });
#[derive(Clone, Copy)] enum E71 { Live(CW<NonZeroU64>), D0(Wrap<HalfDead>, ()) }
suite!(71, s71, E71, [E71::Live(CW(NonZeroU64::new(1).unwrap())), E71::Live(CW(NonZeroU64::new(0xFFFF_FFFF_FFFF_FFFF).unwrap())), E71::Live(CW(NonZeroU64::new(0x1_0000_0000).unwrap())), E71::D0(Wrap(HalfDead::B), ())], |e| match e { E71::Live(w) => { let p: NonZeroU64 = w.0; (p.get() % 1_000_003) as u32 }, E71::D0(..) => 2001 });
#[derive(Clone, Copy)] enum E72 { U1, U3, Live((NonZeroU8,)), U0, U2 }
suite!(72, s72, E72, [E72::U1, E72::U3, E72::Live((NonZeroU8::new(1).unwrap(),)), E72::Live((NonZeroU8::new(255).unwrap(),)), E72::U0, E72::U2], |e| match e { E72::U1 => 1002, E72::U3 => 1004, E72::Live(w) => { let p: NonZeroU8 = w.0; p.get() as u32 }, E72::U0 => 1001, E72::U2 => 1003 });
#[derive(Clone, Copy)] enum E73 { D0((Void,)), Live(ManuallyDrop<NonZeroU16>) }
suite!(73, s73, E73, [E73::Live(ManuallyDrop::new(NonZeroU16::new(1).unwrap())), E73::Live(ManuallyDrop::new(NonZeroU16::new(65535).unwrap()))], |e| match e { E73::D0(..) => 9001, E73::Live(w) => { let p: NonZeroU16 = ManuallyDrop::into_inner(w); p.get() as u32 } });
#[derive(Clone, Copy)] enum E74 { D1(Void, (), PhantomData<u8>), Live(ManuallyDrop<NonZeroU32>), D0((Never, Never)) }
suite!(74, s74, E74, [E74::Live(ManuallyDrop::new(NonZeroU32::new(1).unwrap())), E74::Live(ManuallyDrop::new(NonZeroU32::new(0xFFFF_FFFF).unwrap()))], |e| match e { E74::D1(..) => 9002, E74::Live(w) => { let p: NonZeroU32 = ManuallyDrop::into_inner(w); p.get() }, E74::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E75 { U0, D0((u32, Void)), D1(AllDead), Live(W16) }
suite!(75, s75, E75, [E75::U0, E75::Live(W16::A), E75::Live(W16::B)], |e| match e { E75::U0 => 1001, E75::D0(..) => 9001, E75::D1(..) => 9002, E75::Live(w) => { let p: W16 = w; p as u32 } });
#[derive(Clone, Copy)] enum E76 { Live(&'static u32), U0, D2(()), D0(PhantomData<Void>), D1(Void, [u64; 0]) }
suite!(76, s76, E76, [E76::Live(&5), E76::Live(&SEVEN), E76::U0, E76::D2(()), E76::D0(PhantomData)], |e| match e { E76::Live(w) => { let p: &'static u32 = w; *p }, E76::U0 => 1001, E76::D2(..) => 2003, E76::D0(..) => 2001, E76::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E77 { D0(ManuallyDrop<Void>), Live(Wrap<S16>), D1([[Void; 1]; 1]), D2((HalfDead, Void)), U0 }
suite!(77, s77, E77, [E77::Live(Wrap(S16::A)), E77::Live(Wrap(S16::B)), E77::U0], |e| match e { E77::D0(..) => 9001, E77::Live(w) => { let p: S16 = w.0; p as i32 as u32 }, E77::D1(..) => 9002, E77::D2(..) => 9003, E77::U0 => 1001 });
#[derive(Clone, Copy)] enum E78 { D1(HalfDead2), Live(SL<NonZeroU16>), U0, D0((Void, ())) }
suite!(78, s78, E78, [E78::D1(HalfDead2::A), E78::Live(SL::Live(NonZeroU16::new(1).unwrap())), E78::Live(SL::Live(NonZeroU16::new(65535).unwrap())), E78::U0], |e| match e { E78::D1(..) => 2002, E78::Live(w) => { let p: NonZeroU16 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); p.get() as u32 }, E78::U0 => 1001, E78::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E79 { D1(Void, (), PhantomData<u8>), U0, Live(ZPad<W16>), U1, D0(HalfDead2) }
suite!(79, s79, E79, [E79::U0, E79::Live(ZPad([], W16::A, ())), E79::Live(ZPad([], W16::B, ())), E79::U1, E79::D0(HalfDead2::A)], |e| match e { E79::D1(..) => 9002, E79::U0 => 1001, E79::Live(w) => { let p: W16 = w.1; p as u32 }, E79::U1 => 1002, E79::D0(..) => 2001 });

#[unsafe(no_mangle)]
pub fn rust_entry() {
    s60(); s61(); s62(); s63(); s64(); s65(); s66(); s67(); s68(); s69(); s70(); s71(); s72(); s73(); s74(); s75(); s76(); s77(); s78(); s79();
}
