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
#[derive(Clone, Copy)] enum E60 { U0, U1, U2, Live(CW<Hi2>) }
suite!(60, s60, E60, [E60::U0, E60::U1, E60::U2, E60::Live(CW(Hi2::A)), E60::Live(CW(Hi2::B))], [E60::Live(CW(black_box(Hi2::A))), E60::Live(CW(black_box(Hi2::B)))], |e| match e { E60::U0 => 1001, E60::U1 => 1002, E60::U2 => 1003, E60::Live(w) => { let p: Hi2 = w.0; p as u32 } });
#[derive(Clone, Copy)] enum E61 { Live(((), Small)), D0(Result<(), Void>) }
suite!(61, s61, E61, [E61::Live(((), Small::X)), E61::Live(((), Small::Y)), E61::D0(Ok(()))], [E61::Live(((), black_box(Small::X))), E61::Live(((), black_box(Small::Y)))], |e| match e { E61::Live(w) => { let p: Small = w.1; p as u32 }, E61::D0(..) => 2001 });
#[derive(Clone, Copy)] enum E62 { Live((Two,)) }
suite!(62, s62, E62, [E62::Live((Two::A,)), E62::Live((Two::B,))], [E62::Live((black_box(Two::A),)), E62::Live((black_box(Two::B),))], |e| match e { E62::Live(w) => { let p: Two = w.0; p as u32 + 30 } });
#[derive(Clone, Copy)] enum E63 { D0([(Void,); 3]), D1(Void, (), PhantomData<u8>), U1, U0, Live((Hi2,)), U2 }
suite!(63, s63, E63, [E63::U1, E63::U0, E63::Live((Hi2::A,)), E63::Live((Hi2::B,)), E63::U2], [E63::Live((black_box(Hi2::A),)), E63::Live((black_box(Hi2::B),))], |e| match e { E63::D0(..) => 9001, E63::D1(..) => 9002, E63::U1 => 1002, E63::U0 => 1001, E63::Live(w) => { let p: Hi2 = w.0; p as u32 }, E63::U2 => 1003 });
#[derive(Clone, Copy)] enum E64 { D0(Wrap<HalfDead>, ()), Live((W32,)) }
suite!(64, s64, E64, [E64::D0(Wrap(HalfDead::B), ()), E64::Live((W32::A,)), E64::Live((W32::B,))], [E64::Live((black_box(W32::A),)), E64::Live((black_box(W32::B),))], |e| match e { E64::D0(..) => 2001, E64::Live(w) => { let p: W32 = w.0; p as u32 } });
#[derive(Clone, Copy)] enum E65 { D0([u16; 0], Void), U0, U2, D1(PkDead), U3, Live(Wrap<Sg2>), U1 }
suite!(65, s65, E65, [E65::U0, E65::U2, E65::U3, E65::Live(Wrap(Sg2::A)), E65::Live(Wrap(Sg2::B)), E65::U1], [E65::Live(Wrap(black_box(Sg2::A))), E65::Live(Wrap(black_box(Sg2::B)))], |e| match e { E65::D0(..) => 9001, E65::U0 => 1001, E65::U2 => 1003, E65::D1(..) => 9002, E65::U3 => 1004, E65::Live(w) => { let p: Sg2 = w.0; p as i32 as u32 }, E65::U1 => 1002 });
#[derive(Clone, Copy)] enum E66 { D0(Wrap<HalfDead>, ()), Live(ZPad<WU>) }
suite!(66, s66, E66, [E66::D0(Wrap(HalfDead::B), ()), E66::Live(ZPad([], WU::A, ())), E66::Live(ZPad([], WU::B, ()))], [E66::Live(ZPad([], black_box(WU::A), ())), E66::Live(ZPad([], black_box(WU::B), ()))], |e| match e { E66::D0(..) => 2001, E66::Live(w) => { let p: WU = w.1; p as usize as u32 } });
#[derive(Clone, Copy)] enum E67 { Live(Result<Never, W16>), U0, U1 }
suite!(67, s67, E67, [E67::Live(Err(W16::A)), E67::Live(Err(W16::B)), E67::U0, E67::U1], [E67::Live(Err(black_box(W16::A))), E67::Live(Err(black_box(W16::B)))], |e| match e { E67::Live(w) => { let p: W16 = (match w { Err(x) => x, Ok(_) => loop {} }); p as u32 }, E67::U0 => 1001, E67::U1 => 1002 });
#[derive(Clone, Copy)] enum E68 { Live(Wrap<Tr<((), WU)>>), D0((), Infallible), D1((Void, ())), D2(HalfDead2), U0 }
suite!(68, s68, E68, [E68::Live(Wrap(Tr(((), WU::A)))), E68::Live(Wrap(Tr(((), WU::B)))), E68::D2(HalfDead2::A), E68::U0], [E68::Live(Wrap(Tr(((), black_box(WU::A))))), E68::Live(Wrap(Tr(((), black_box(WU::B)))))], |e| match e { E68::Live(w) => { let p: WU = w.0.0.1; p as usize as u32 }, E68::D0(..) => 9001, E68::D1(..) => 9002, E68::D2(..) => 2003, E68::U0 => 1001 });
#[derive(Clone, Copy)] enum E69 { Live((NonZeroU16,)), D1(u8, Void), D0([Void; 2]), U2, U0, U1 }
suite!(69, s69, E69, [E69::Live((NonZeroU16::new(1).unwrap(),)), E69::Live((NonZeroU16::new(65535).unwrap(),)), E69::U2, E69::U0, E69::U1], [E69::Live((black_box(NonZeroU16::new(1).unwrap()),)), E69::Live((black_box(NonZeroU16::new(65535).unwrap()),))], |e| match e { E69::Live(w) => { let p: NonZeroU16 = w.0; p.get() as u32 }, E69::D1(..) => 9002, E69::D0(..) => 9001, E69::U2 => 1003, E69::U0 => 1001, E69::U1 => 1002 });
#[derive(Clone, Copy)] enum E70 { U0, U3, D1(Infallible), U2, D0(&'static Void, Void), U1, Live(CW<NonZeroI8>), U4, U5 }
suite!(70, s70, E70, [E70::U0, E70::U3, E70::U2, E70::U1, E70::Live(CW(NonZeroI8::new(-1).unwrap())), E70::Live(CW(NonZeroI8::new(127).unwrap())), E70::U4, E70::U5], [E70::Live(CW(black_box(NonZeroI8::new(-1).unwrap()))), E70::Live(CW(black_box(NonZeroI8::new(127).unwrap())))], |e| match e { E70::U0 => 1001, E70::U3 => 1004, E70::D1(..) => 9002, E70::U2 => 1003, E70::D0(..) => 9001, E70::U1 => 1002, E70::Live(w) => { let p: NonZeroI8 = w.0; p.get() as i32 as u32 }, E70::U4 => 1005, E70::U5 => 1006 });
#[derive(Clone, Copy)] enum E71 { Live(ZPad<S16>), D1([u16; 0], Void), U0, D0(HalfDead), D2([Void; 2]) }
suite!(71, s71, E71, [E71::Live(ZPad([], S16::A, ())), E71::Live(ZPad([], S16::B, ())), E71::U0, E71::D0(HalfDead::B)], [E71::Live(ZPad([], black_box(S16::A), ())), E71::Live(ZPad([], black_box(S16::B), ()))], |e| match e { E71::Live(w) => { let p: S16 = w.1; p as i32 as u32 }, E71::D1(..) => 9002, E71::U0 => 1001, E71::D0(..) => 2001, E71::D2(..) => 9003 });
#[derive(Clone, Copy)] enum E72 { Live(Result<Hi3, Void>), D0(Result<(), Void>) }
suite!(72, s72, E72, [E72::Live(Ok(Hi3::A)), E72::Live(Ok(Hi3::B)), E72::Live(Ok(Hi3::C)), E72::D0(Ok(()))], [E72::Live(Ok(black_box(Hi3::A))), E72::Live(Ok(black_box(Hi3::B))), E72::Live(Ok(black_box(Hi3::C)))], |e| match e { E72::Live(w) => { let p: Hi3 = (match w { Ok(x) => x, Err(_) => loop {} }); p as u32 }, E72::D0(..) => 2001 });
#[derive(Clone, Copy)] enum E73 { D0(Option<Void>), Live(Wrap<NonZeroI8>) }
suite!(73, s73, E73, [E73::D0(None), E73::Live(Wrap(NonZeroI8::new(-1).unwrap())), E73::Live(Wrap(NonZeroI8::new(127).unwrap()))], [E73::Live(Wrap(black_box(NonZeroI8::new(-1).unwrap()))), E73::Live(Wrap(black_box(NonZeroI8::new(127).unwrap())))], |e| match e { E73::D0(..) => 2001, E73::Live(w) => { let p: NonZeroI8 = w.0; p.get() as i32 as u32 } });
#[derive(Clone, Copy)] enum E74 { D0(HalfDead), Live([Sg2; 1]), D1(()), U0 }
suite!(74, s74, E74, [E74::D0(HalfDead::B), E74::Live([Sg2::A]), E74::Live([Sg2::B]), E74::D1(()), E74::U0], [E74::Live([black_box(Sg2::A)]), E74::Live([black_box(Sg2::B)])], |e| match e { E74::D0(..) => 2001, E74::Live(w) => { let p: Sg2 = w[0]; p as i32 as u32 }, E74::D1(..) => 2002, E74::U0 => 1001 });
#[derive(Clone, Copy)] enum E75 { Live(Wrap<Tr<((), NonZeroI16)>>), D0(PkDead), U0 }
suite!(75, s75, E75, [E75::Live(Wrap(Tr(((), NonZeroI16::new(-1).unwrap())))), E75::Live(Wrap(Tr(((), NonZeroI16::new(300).unwrap())))), E75::U0], [E75::Live(Wrap(Tr(((), black_box(NonZeroI16::new(-1).unwrap()))))), E75::Live(Wrap(Tr(((), black_box(NonZeroI16::new(300).unwrap())))))], |e| match e { E75::Live(w) => { let p: NonZeroI16 = w.0.0.1; p.get() as i32 as u32 }, E75::D0(..) => 9001, E75::U0 => 1001 });
#[derive(Clone, Copy)] enum E76 { Live(SL3<char>), U0, U2, D1(()), D0(ManuallyDrop<Void>), U1 }
suite!(76, s76, E76, [E76::Live(SL3::Live('a')), E76::Live(SL3::Live('\u{10FFFF}')), E76::Live(SL3::Live('\0')), E76::U0, E76::U2, E76::D1(()), E76::U1], [E76::Live(SL3::Live(black_box('a'))), E76::Live(SL3::Live(black_box('\u{10FFFF}'))), E76::Live(SL3::Live(black_box('\0')))], |e| match e { E76::Live(w) => { let p: char = (match w { SL3::Live(x) => x, _ => loop {} }); p as u32 }, E76::U0 => 1001, E76::U2 => 1003, E76::D1(..) => 2002, E76::D0(..) => 9001, E76::U1 => 1002 });
#[derive(Clone, Copy)] enum E77 { D0((Void, ())), D1(Wrap<HalfDead>, ()), Live(Result<Option<char>, Void>) }
suite!(77, s77, E77, [E77::D1(Wrap(HalfDead::B), ()), E77::Live(Ok(Some('z'))), E77::Live(Ok(None))], [E77::Live(Ok(black_box(Some('z')))), E77::Live(Ok(black_box(None)))], |e| match e { E77::D0(..) => 9001, E77::D1(..) => 2002, E77::Live(w) => { let p: Option<char> = (match w { Ok(x) => x, Err(_) => loop {} }); match p { Some(c) => c as u32, None => 48 } } });
#[derive(Clone, Copy)] enum E78 { U0, D0(()), Live(Result<NonZeroU64, Void>), D1(AllDead) }
suite!(78, s78, E78, [E78::U0, E78::D0(()), E78::Live(Ok(NonZeroU64::new(1).unwrap())), E78::Live(Ok(NonZeroU64::new(0xFFFF_FFFF_FFFF_FFFF).unwrap())), E78::Live(Ok(NonZeroU64::new(0x1_0000_0000).unwrap()))], [E78::Live(Ok(black_box(NonZeroU64::new(1).unwrap()))), E78::Live(Ok(black_box(NonZeroU64::new(0xFFFF_FFFF_FFFF_FFFF).unwrap()))), E78::Live(Ok(black_box(NonZeroU64::new(0x1_0000_0000).unwrap())))], |e| match e { E78::U0 => 1001, E78::D0(..) => 2001, E78::Live(w) => { let p: NonZeroU64 = (match w { Ok(x) => x, Err(_) => loop {} }); (p.get() % 1_000_003) as u32 }, E78::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E79 { U0, Live(Result<Never, bool>), U2, U1, U3 }
suite!(79, s79, E79, [E79::U0, E79::Live(Err(true)), E79::Live(Err(false)), E79::U2, E79::U1, E79::U3], [E79::Live(Err(black_box(true))), E79::Live(Err(black_box(false)))], |e| match e { E79::U0 => 1001, E79::Live(w) => { let p: bool = (match w { Err(x) => x, Ok(_) => loop {} }); 10 + p as u32 }, E79::U2 => 1003, E79::U1 => 1002, E79::U3 => 1004 });

#[unsafe(no_mangle)]
pub fn rust_entry() {
    s60(); s61(); s62(); s63(); s64(); s65(); s66(); s67(); s68(); s69(); s70(); s71(); s72(); s73(); s74(); s75(); s76(); s77(); s78(); s79();
}
