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
#[derive(Clone, Copy)] enum E100 { Live(SL<Option<bool>>), D1(HalfDead), D0([(Void,); 3]) }
suite!(100, s100, E100, [E100::Live(SL::Live(Some(true))), E100::Live(SL::Live(Some(false))), E100::Live(SL::Live(None)), E100::D1(HalfDead::B)], [E100::Live(SL::Live(black_box(Some(true)))), E100::Live(SL::Live(black_box(Some(false)))), E100::Live(SL::Live(black_box(None)))], |e| match e { E100::Live(w) => { let p: Option<bool> = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); match p { Some(b) => 40 + b as u32, None => 49 } }, E100::D1(..) => 2002, E100::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E101 { Live([NonZeroI16; 1]), U5, U3, U0, U4, U2, D0(([Void; 0], Void)), U1 }
suite!(101, s101, E101, [E101::Live([NonZeroI16::new(-1).unwrap()]), E101::Live([NonZeroI16::new(300).unwrap()]), E101::U5, E101::U3, E101::U0, E101::U4, E101::U2, E101::U1], [E101::Live([black_box(NonZeroI16::new(-1).unwrap())]), E101::Live([black_box(NonZeroI16::new(300).unwrap())])], |e| match e { E101::Live(w) => { let p: NonZeroI16 = w[0]; p.get() as i32 as u32 }, E101::U5 => 1006, E101::U3 => 1004, E101::U0 => 1001, E101::U4 => 1005, E101::U2 => 1003, E101::D0(..) => 9001, E101::U1 => 1002 });
#[derive(Clone, Copy)] enum E102 { U1, U2, U3, D1(Wrap<HalfDead>, ()), U4, U0, Live(CW<Small>), D0((Void, ())), U5, D2(Wrap<Wrap<(Void, ())>>) }
suite!(102, s102, E102, [E102::U1, E102::U2, E102::U3, E102::D1(Wrap(HalfDead::B), ()), E102::U4, E102::U0, E102::Live(CW(Small::X)), E102::Live(CW(Small::Y)), E102::U5], [E102::Live(CW(black_box(Small::X))), E102::Live(CW(black_box(Small::Y)))], |e| match e { E102::U1 => 1002, E102::U2 => 1003, E102::U3 => 1004, E102::D1(..) => 2002, E102::U4 => 1005, E102::U0 => 1001, E102::Live(w) => { let p: Small = w.0; p as u32 }, E102::D0(..) => 9001, E102::U5 => 1006, E102::D2(..) => 9003 });
#[derive(Clone, Copy)] enum E103 { D1((u32, Void)), D0([u32; 0]), Live(SL<Wrap<SL3<NonZeroU64>>>) }
suite!(103, s103, E103, [E103::D0([]), E103::Live(SL::Live(Wrap(SL3::Live(NonZeroU64::new(1).unwrap())))), E103::Live(SL::Live(Wrap(SL3::Live(NonZeroU64::new(0xFFFF_FFFF_FFFF_FFFF).unwrap())))), E103::Live(SL::Live(Wrap(SL3::Live(NonZeroU64::new(0x1_0000_0000).unwrap()))))], [E103::Live(SL::Live(Wrap(SL3::Live(black_box(NonZeroU64::new(1).unwrap()))))), E103::Live(SL::Live(Wrap(SL3::Live(black_box(NonZeroU64::new(0xFFFF_FFFF_FFFF_FFFF).unwrap()))))), E103::Live(SL::Live(Wrap(SL3::Live(black_box(NonZeroU64::new(0x1_0000_0000).unwrap())))))], |e| match e { E103::D1(..) => 9002, E103::D0(..) => 2001, E103::Live(w) => { let p: NonZeroU64 = (match w { SL::Live(Wrap(SL3::Live(x))) => x, _ => loop {} }); (p.get() % 1_000_003) as u32 } });
#[derive(Clone, Copy)] enum E104 { Live((Hi2, PhantomData<Void>, [Void; 0])) }
suite!(104, s104, E104, [E104::Live((Hi2::A, PhantomData, [])), E104::Live((Hi2::B, PhantomData, []))], [E104::Live((black_box(Hi2::A), PhantomData, [])), E104::Live((black_box(Hi2::B), PhantomData, []))], |e| match e { E104::Live(w) => { let p: Hi2 = w.0; p as u32 } });
#[derive(Clone, Copy)] enum E105 { Live([NonZeroU32; 1]), D0((), Infallible) }
suite!(105, s105, E105, [E105::Live([NonZeroU32::new(1).unwrap()]), E105::Live([NonZeroU32::new(0xFFFF_FFFF).unwrap()])], [E105::Live([black_box(NonZeroU32::new(1).unwrap())]), E105::Live([black_box(NonZeroU32::new(0xFFFF_FFFF).unwrap())])], |e| match e { E105::Live(w) => { let p: NonZeroU32 = w[0]; p.get() }, E105::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E106 { U0, Live(Sg3), D0([u32; 0], Void) }
suite!(106, s106, E106, [E106::U0, E106::Live(Sg3::A), E106::Live(Sg3::B)], [E106::Live(black_box(Sg3::A)), E106::Live(black_box(Sg3::B))], |e| match e { E106::U0 => 1001, E106::Live(w) => { let p: Sg3 = w; p as i32 as u32 }, E106::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E107 { Live(Tr<Option<Hi2>>) }
suite!(107, s107, E107, [E107::Live(Tr(Some(Hi2::A))), E107::Live(Tr(Some(Hi2::B))), E107::Live(Tr(None))], [E107::Live(Tr(black_box(Some(Hi2::A)))), E107::Live(Tr(black_box(Some(Hi2::B)))), E107::Live(Tr(black_box(None)))], |e| match e { E107::Live(w) => { let p: Option<Hi2> = w.0; match p { Some(h) => h as u32, None => 47 } } });
#[derive(Clone, Copy)] enum E108 { Live(((), NonZeroU64)), U0, D0(PhantomData<Void>) }
suite!(108, s108, E108, [E108::Live(((), NonZeroU64::new(1).unwrap())), E108::Live(((), NonZeroU64::new(0xFFFF_FFFF_FFFF_FFFF).unwrap())), E108::Live(((), NonZeroU64::new(0x1_0000_0000).unwrap())), E108::U0, E108::D0(PhantomData)], [E108::Live(((), black_box(NonZeroU64::new(1).unwrap()))), E108::Live(((), black_box(NonZeroU64::new(0xFFFF_FFFF_FFFF_FFFF).unwrap()))), E108::Live(((), black_box(NonZeroU64::new(0x1_0000_0000).unwrap())))], |e| match e { E108::Live(w) => { let p: NonZeroU64 = w.1; (p.get() % 1_000_003) as u32 }, E108::U0 => 1001, E108::D0(..) => 2001 });
#[derive(Clone, Copy)] enum E109 { D1(Infallible), U1, Live(SL<Wrap<SL3<Ordering>>>), D0((), Infallible), U0 }
suite!(109, s109, E109, [E109::U1, E109::Live(SL::Live(Wrap(SL3::Live(Ordering::Less)))), E109::Live(SL::Live(Wrap(SL3::Live(Ordering::Equal)))), E109::Live(SL::Live(Wrap(SL3::Live(Ordering::Greater)))), E109::U0], [E109::Live(SL::Live(Wrap(SL3::Live(black_box(Ordering::Less))))), E109::Live(SL::Live(Wrap(SL3::Live(black_box(Ordering::Equal))))), E109::Live(SL::Live(Wrap(SL3::Live(black_box(Ordering::Greater)))))], |e| match e { E109::D1(..) => 9002, E109::U1 => 1002, E109::Live(w) => { let p: Ordering = (match w { SL::Live(Wrap(SL3::Live(x))) => x, _ => loop {} }); p as i32 as u32 }, E109::D0(..) => 9001, E109::U0 => 1001 });
#[derive(Clone, Copy)] enum E110 { Live(W64), D0(Wrap<Void>) }
suite!(110, s110, E110, [E110::Live(W64::A), E110::Live(W64::B)], [E110::Live(black_box(W64::A)), E110::Live(black_box(W64::B))], |e| match e { E110::Live(w) => { let p: W64 = w; (p as u64 % 1_000_003) as u32 }, E110::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E111 { U2, U0, U1, Live(SL3<NonZeroI8>), D0(Wrap<HalfDead>, ()) }
suite!(111, s111, E111, [E111::U2, E111::U0, E111::U1, E111::Live(SL3::Live(NonZeroI8::new(-1).unwrap())), E111::Live(SL3::Live(NonZeroI8::new(127).unwrap())), E111::D0(Wrap(HalfDead::B), ())], [E111::Live(SL3::Live(black_box(NonZeroI8::new(-1).unwrap()))), E111::Live(SL3::Live(black_box(NonZeroI8::new(127).unwrap())))], |e| match e { E111::U2 => 1003, E111::U0 => 1001, E111::U1 => 1002, E111::Live(w) => { let p: NonZeroI8 = (match w { SL3::Live(x) => x, _ => loop {} }); p.get() as i32 as u32 }, E111::D0(..) => 2001 });
#[derive(Clone, Copy)] enum E112 { D0(Result<Void, Never>), U1, U0, Live((Option<bool>, PhantomData<Void>, [Void; 0])), U2, D1(AllDead) }
suite!(112, s112, E112, [E112::U1, E112::U0, E112::Live((Some(true), PhantomData, [])), E112::Live((Some(false), PhantomData, [])), E112::Live((None, PhantomData, [])), E112::U2], [E112::Live((black_box(Some(true)), PhantomData, [])), E112::Live((black_box(Some(false)), PhantomData, [])), E112::Live((black_box(None), PhantomData, []))], |e| match e { E112::D0(..) => 9001, E112::U1 => 1002, E112::U0 => 1001, E112::Live(w) => { let p: Option<bool> = w.0; match p { Some(b) => 40 + b as u32, None => 49 } }, E112::U2 => 1003, E112::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E113 { D1([[Void; 1]; 1]), Live(ManuallyDrop<NonZeroI8>), D0(([Void; 0], Void)) }
suite!(113, s113, E113, [E113::Live(ManuallyDrop::new(NonZeroI8::new(-1).unwrap())), E113::Live(ManuallyDrop::new(NonZeroI8::new(127).unwrap()))], [E113::Live(ManuallyDrop::new(black_box(NonZeroI8::new(-1).unwrap()))), E113::Live(ManuallyDrop::new(black_box(NonZeroI8::new(127).unwrap())))], |e| match e { E113::D1(..) => 9002, E113::Live(w) => { let p: NonZeroI8 = ManuallyDrop::into_inner(w); p.get() as i32 as u32 }, E113::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E114 { Live(Result<NonZeroU32, Void>), D0((HalfDead, Void)), D1([u32; 0]) }
suite!(114, s114, E114, [E114::Live(Ok(NonZeroU32::new(1).unwrap())), E114::Live(Ok(NonZeroU32::new(0xFFFF_FFFF).unwrap())), E114::D1([])], [E114::Live(Ok(black_box(NonZeroU32::new(1).unwrap()))), E114::Live(Ok(black_box(NonZeroU32::new(0xFFFF_FFFF).unwrap())))], |e| match e { E114::Live(w) => { let p: NonZeroU32 = (match w { Ok(x) => x, Err(_) => loop {} }); p.get() }, E114::D0(..) => 9001, E114::D1(..) => 2002 });
#[derive(Clone, Copy)] enum E115 { U4, U3, D0(Wrap<Void>), U0, U5, Live(SL<Wrap<SL3<NonZeroU8>>>), U1, U2 }
suite!(115, s115, E115, [E115::U4, E115::U3, E115::U0, E115::U5, E115::Live(SL::Live(Wrap(SL3::Live(NonZeroU8::new(1).unwrap())))), E115::Live(SL::Live(Wrap(SL3::Live(NonZeroU8::new(255).unwrap())))), E115::U1, E115::U2], [E115::Live(SL::Live(Wrap(SL3::Live(black_box(NonZeroU8::new(1).unwrap()))))), E115::Live(SL::Live(Wrap(SL3::Live(black_box(NonZeroU8::new(255).unwrap())))))], |e| match e { E115::U4 => 1005, E115::U3 => 1004, E115::D0(..) => 9001, E115::U0 => 1001, E115::U5 => 1006, E115::Live(w) => { let p: NonZeroU8 = (match w { SL::Live(Wrap(SL3::Live(x))) => x, _ => loop {} }); p.get() as u32 }, E115::U1 => 1002, E115::U2 => 1003 });
#[derive(Clone, Copy)] enum E116 { D0([(Void,); 0]), Live(Pk<Small>), U1, U0 }
suite!(116, s116, E116, [E116::D0([]), E116::Live(Pk(Small::X)), E116::Live(Pk(Small::Y)), E116::U1, E116::U0], [E116::Live(Pk(black_box(Small::X))), E116::Live(Pk(black_box(Small::Y)))], |e| match e { E116::D0(..) => 2001, E116::Live(w) => { let p: Small = { w.0 }; p as u32 }, E116::U1 => 1002, E116::U0 => 1001 });
#[derive(Clone, Copy)] enum E117 { Live(ManuallyDrop<Sg3>), U1, D0(&'static Void, Void), U2, U0 }
suite!(117, s117, E117, [E117::Live(ManuallyDrop::new(Sg3::A)), E117::Live(ManuallyDrop::new(Sg3::B)), E117::U1, E117::U2, E117::U0], [E117::Live(ManuallyDrop::new(black_box(Sg3::A))), E117::Live(ManuallyDrop::new(black_box(Sg3::B)))], |e| match e { E117::Live(w) => { let p: Sg3 = ManuallyDrop::into_inner(w); p as i32 as u32 }, E117::U1 => 1002, E117::D0(..) => 9001, E117::U2 => 1003, E117::U0 => 1001 });
#[derive(Clone, Copy)] enum E118 { U0, U1, Live(S32) }
suite!(118, s118, E118, [E118::U0, E118::U1, E118::Live(S32::A), E118::Live(S32::B)], [E118::Live(black_box(S32::A)), E118::Live(black_box(S32::B))], |e| match e { E118::U0 => 1001, E118::U1 => 1002, E118::Live(w) => { let p: S32 = w; p as i32 as u32 } });
#[derive(Clone, Copy)] enum E119 { D0(Infallible), U0, U1, Live(Tr<char>), D1(Aligned) }
suite!(119, s119, E119, [E119::U0, E119::U1, E119::Live(Tr('a')), E119::Live(Tr('\u{10FFFF}')), E119::Live(Tr('\0'))], [E119::Live(Tr(black_box('a'))), E119::Live(Tr(black_box('\u{10FFFF}'))), E119::Live(Tr(black_box('\0')))], |e| match e { E119::D0(..) => 9001, E119::U0 => 1001, E119::U1 => 1002, E119::Live(w) => { let p: char = w.0; p as u32 }, E119::D1(..) => 9002 });

#[unsafe(no_mangle)]
pub fn rust_entry() {
    s100(); s101(); s102(); s103(); s104(); s105(); s106(); s107(); s108(); s109(); s110(); s111(); s112(); s113(); s114(); s115(); s116(); s117(); s118(); s119();
}
