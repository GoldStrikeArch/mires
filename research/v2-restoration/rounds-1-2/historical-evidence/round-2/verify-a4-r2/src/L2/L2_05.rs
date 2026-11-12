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
#[derive(Clone, Copy)] enum E100 { U1, D1(Never), U3, U0, U2, D0(&'static Void, Void), Live(Result<Never, Hi3>) }
suite!(100, s100, E100, [E100::U1, E100::U3, E100::U0, E100::U2, E100::Live(Err(Hi3::A)), E100::Live(Err(Hi3::B)), E100::Live(Err(Hi3::C))], |e| match e { E100::U1 => 1002, E100::D1(..) => 9002, E100::U3 => 1004, E100::U0 => 1001, E100::U2 => 1003, E100::D0(..) => 9001, E100::Live(w) => { let p: Hi3 = (match w { Err(x) => x, Ok(_) => loop {} }); p as u32 } });
#[derive(Clone, Copy)] enum E101 { D0(HalfDead), Live(Tr<bool>), U0 }
suite!(101, s101, E101, [E101::D0(HalfDead::B), E101::Live(Tr(true)), E101::Live(Tr(false)), E101::U0], |e| match e { E101::D0(..) => 2001, E101::Live(w) => { let p: bool = w.0; 10 + p as u32 }, E101::U0 => 1001 });
#[derive(Clone, Copy)] enum E102 { U0, D1(Wrap<HalfDead>, ()), D2(Void, (), PhantomData<u8>), Live(SL<Wrap<SL3<bool>>>), D0((HalfDead, Void)) }
suite!(102, s102, E102, [E102::U0, E102::D1(Wrap(HalfDead::B), ()), E102::Live(SL::Live(Wrap(SL3::Live(true)))), E102::Live(SL::Live(Wrap(SL3::Live(false))))], |e| match e { E102::U0 => 1001, E102::D1(..) => 2002, E102::D2(..) => 9003, E102::Live(w) => { let p: bool = (match w { SL::Live(Wrap(SL3::Live(x))) => x, _ => loop {} }); 10 + p as u32 }, E102::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E103 { Live(Tr<Three>), D0(&'static Void, Void) }
suite!(103, s103, E103, [E103::Live(Tr(Three::A)), E103::Live(Tr(Three::B)), E103::Live(Tr(Three::C))], |e| match e { E103::Live(w) => { let p: Three = w.0; p as u32 + 20 }, E103::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E104 { D0((Void, ())), U0, U1, Live(ManuallyDrop<bool>) }
suite!(104, s104, E104, [E104::U0, E104::U1, E104::Live(ManuallyDrop::new(true)), E104::Live(ManuallyDrop::new(false))], |e| match e { E104::D0(..) => 9001, E104::U0 => 1001, E104::U1 => 1002, E104::Live(w) => { let p: bool = ManuallyDrop::into_inner(w); 10 + p as u32 } });
#[derive(Clone, Copy)] enum E105 { U0, Live((W64, PhantomData<Void>, [Void; 0])), D1(Never), D2(Infallible), D0([[Void; 1]; 1]), U1 }
suite!(105, s105, E105, [E105::U0, E105::Live((W64::A, PhantomData, [])), E105::Live((W64::B, PhantomData, [])), E105::U1], |e| match e { E105::U0 => 1001, E105::Live(w) => { let p: W64 = w.0; (p as u64 % 1_000_003) as u32 }, E105::D1(..) => 9002, E105::D2(..) => 9003, E105::D0(..) => 9001, E105::U1 => 1002 });
#[derive(Clone, Copy)] enum E106 { D0((HalfDead, Void)), U0, U1, D2(Wrap<AllDead>), Live(Sg2), D1(NamedDead) }
suite!(106, s106, E106, [E106::U0, E106::U1, E106::Live(Sg2::A), E106::Live(Sg2::B)], |e| match e { E106::D0(..) => 9001, E106::U0 => 1001, E106::U1 => 1002, E106::D2(..) => 9003, E106::Live(w) => { let p: Sg2 = w; p as i32 as u32 }, E106::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E107 { D0(u8, Void), U0, D1(PkDead), Live(CW<&'static u32>) }
suite!(107, s107, E107, [E107::U0, E107::Live(CW(&5)), E107::Live(CW(&SEVEN))], |e| match e { E107::D0(..) => 9001, E107::U0 => 1001, E107::D1(..) => 9002, E107::Live(w) => { let p: &'static u32 = w.0; *p } });
#[derive(Clone, Copy)] enum E108 { D0((Never, Never)), Live(char), D1((), Infallible), U0 }
suite!(108, s108, E108, [E108::Live('a'), E108::Live('\u{10FFFF}'), E108::Live('\0'), E108::U0], |e| match e { E108::D0(..) => 9001, E108::Live(w) => { let p: char = w; p as u32 }, E108::D1(..) => 9002, E108::U0 => 1001 });
#[derive(Clone, Copy)] enum E109 { D0(HalfDead2), Live(SL<Wrap<SL3<Option<Hi2>>>>) }
suite!(109, s109, E109, [E109::D0(HalfDead2::A), E109::Live(SL::Live(Wrap(SL3::Live(Some(Hi2::A))))), E109::Live(SL::Live(Wrap(SL3::Live(Some(Hi2::B))))), E109::Live(SL::Live(Wrap(SL3::Live(None))))], |e| match e { E109::D0(..) => 2001, E109::Live(w) => { let p: Option<Hi2> = (match w { SL::Live(Wrap(SL3::Live(x))) => x, _ => loop {} }); match p { Some(h) => h as u32, None => 47 } } });
#[derive(Clone, Copy)] enum E110 { D1(Result<(), Void>), D0([u32; 0]), Live(Wrap<NonZeroU32>), D2(DeepDead) }
suite!(110, s110, E110, [E110::D1(Ok(())), E110::D0([]), E110::Live(Wrap(NonZeroU32::new(1).unwrap())), E110::Live(Wrap(NonZeroU32::new(0xFFFF_FFFF).unwrap()))], |e| match e { E110::D1(..) => 2002, E110::D0(..) => 2001, E110::Live(w) => { let p: NonZeroU32 = w.0; p.get() }, E110::D2(..) => 9003 });
#[derive(Clone, Copy)] enum E111 { U1, U3, Live(Result<Never, bool>), U2, U4, U0, D0((Never, Never)), D1((Void,)), U5 }
suite!(111, s111, E111, [E111::U1, E111::U3, E111::Live(Err(true)), E111::Live(Err(false)), E111::U2, E111::U4, E111::U0, E111::U5], |e| match e { E111::U1 => 1002, E111::U3 => 1004, E111::Live(w) => { let p: bool = (match w { Err(x) => x, Ok(_) => loop {} }); 10 + p as u32 }, E111::U2 => 1003, E111::U4 => 1005, E111::U0 => 1001, E111::D0(..) => 9001, E111::D1(..) => 9002, E111::U5 => 1006 });
#[derive(Clone, Copy)] enum E112 { Live(Wrap<Tr<((), Option<Option<Three>>)>>), D0(Void, [u64; 0]), U0, D1([Void; 0]) }
suite!(112, s112, E112, [E112::Live(Wrap(Tr(((), Some(Some(Three::C)))))), E112::Live(Wrap(Tr(((), Some(None))))), E112::Live(Wrap(Tr(((), None)))), E112::U0, E112::D1([])], |e| match e { E112::Live(w) => { let p: Option<Option<Three>> = w.0.0.1; match p { Some(Some(t)) => t as u32 + 60, Some(None) => 46, None => 45 } }, E112::D0(..) => 9001, E112::U0 => 1001, E112::D1(..) => 2002 });
#[derive(Clone, Copy)] enum E113 { U1, D0([u32; 0], Void), U0, Live(Tr<W64>) }
suite!(113, s113, E113, [E113::U1, E113::U0, E113::Live(Tr(W64::A)), E113::Live(Tr(W64::B))], |e| match e { E113::U1 => 1002, E113::D0(..) => 9001, E113::U0 => 1001, E113::Live(w) => { let p: W64 = w.0; (p as u64 % 1_000_003) as u32 } });
#[derive(Clone, Copy)] enum E114 { U1, U0, Live(SL<S64>), D1((HalfDead, Void)), D0(CDead) }
suite!(114, s114, E114, [E114::U1, E114::U0, E114::Live(SL::Live(S64::A)), E114::Live(SL::Live(S64::B))], |e| match e { E114::U1 => 1002, E114::U0 => 1001, E114::Live(w) => { let p: S64 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); (p as i64 as u64 % 1_000_003) as u32 }, E114::D1(..) => 9002, E114::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E115 { D1(Never), U1, U0, D2(HalfDead2), D0(&'static Void, Void), Live(Named<Ordering>) }
suite!(115, s115, E115, [E115::U1, E115::U0, E115::D2(HalfDead2::A), E115::Live(Named { z: (), v: Ordering::Less, p: PhantomData }), E115::Live(Named { z: (), v: Ordering::Equal, p: PhantomData }), E115::Live(Named { z: (), v: Ordering::Greater, p: PhantomData })], |e| match e { E115::D1(..) => 9002, E115::U1 => 1002, E115::U0 => 1001, E115::D2(..) => 2003, E115::D0(..) => 9001, E115::Live(w) => { let p: Ordering = w.v; p as i32 as u32 } });
#[derive(Clone, Copy)] enum E116 { Live(ZPad<S32>), U0, U1, D0((Never, Never)) }
suite!(116, s116, E116, [E116::Live(ZPad([], S32::A, ())), E116::Live(ZPad([], S32::B, ())), E116::U0, E116::U1], |e| match e { E116::Live(w) => { let p: S32 = w.1; p as i32 as u32 }, E116::U0 => 1001, E116::U1 => 1002, E116::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E117 { Live(SL<Wrap<SL3<S16>>>), U1, D0(Wrap<AllDead>), D2((), Infallible), U0, D1(PhantomData<Void>), U3, U2 }
suite!(117, s117, E117, [E117::Live(SL::Live(Wrap(SL3::Live(S16::A)))), E117::Live(SL::Live(Wrap(SL3::Live(S16::B)))), E117::U1, E117::U0, E117::D1(PhantomData), E117::U3, E117::U2], |e| match e { E117::Live(w) => { let p: S16 = (match w { SL::Live(Wrap(SL3::Live(x))) => x, _ => loop {} }); p as i32 as u32 }, E117::U1 => 1002, E117::D0(..) => 9001, E117::D2(..) => 9003, E117::U0 => 1001, E117::D1(..) => 2002, E117::U3 => 1004, E117::U2 => 1003 });
#[derive(Clone, Copy)] enum E118 { Live(Named<WU>), U0, U1, U2 }
suite!(118, s118, E118, [E118::Live(Named { z: (), v: WU::A, p: PhantomData }), E118::Live(Named { z: (), v: WU::B, p: PhantomData }), E118::U0, E118::U1, E118::U2], |e| match e { E118::Live(w) => { let p: WU = w.v; p as usize as u32 }, E118::U0 => 1001, E118::U1 => 1002, E118::U2 => 1003 });
#[derive(Clone, Copy)] enum E119 { Live(Result<Never, W64>), U0, U1, U3, U2 }
suite!(119, s119, E119, [E119::Live(Err(W64::A)), E119::Live(Err(W64::B)), E119::U0, E119::U1, E119::U3, E119::U2], |e| match e { E119::Live(w) => { let p: W64 = (match w { Err(x) => x, Ok(_) => loop {} }); (p as u64 % 1_000_003) as u32 }, E119::U0 => 1001, E119::U1 => 1002, E119::U3 => 1004, E119::U2 => 1003 });

#[unsafe(no_mangle)]
pub fn rust_entry() {
    s100(); s101(); s102(); s103(); s104(); s105(); s106(); s107(); s108(); s109(); s110(); s111(); s112(); s113(); s114(); s115(); s116(); s117(); s118(); s119();
}
