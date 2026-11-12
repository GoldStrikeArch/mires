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
#[derive(Clone, Copy)] enum E120 { D0(Wrap<AllDead>), U1, Live(ManuallyDrop<Option<Hi2>>), U0 }
suite!(120, s120, E120, [E120::U1, E120::Live(ManuallyDrop::new(Some(Hi2::A))), E120::Live(ManuallyDrop::new(Some(Hi2::B))), E120::Live(ManuallyDrop::new(None)), E120::U0], |e| match e { E120::D0(..) => 9001, E120::U1 => 1002, E120::Live(w) => { let p: Option<Hi2> = ManuallyDrop::into_inner(w); match p { Some(h) => h as u32, None => 47 } }, E120::U0 => 1001 });
#[derive(Clone, Copy)] enum E121 { Live(Result<Sg3, Void>), D1(ManuallyDrop<Void>), U0, D0([u32; 0]) }
suite!(121, s121, E121, [E121::Live(Ok(Sg3::A)), E121::Live(Ok(Sg3::B)), E121::U0, E121::D0([])], |e| match e { E121::Live(w) => { let p: Sg3 = (match w { Ok(x) => x, Err(_) => loop {} }); p as i32 as u32 }, E121::D1(..) => 9002, E121::U0 => 1001, E121::D0(..) => 2001 });
#[derive(Clone, Copy)] enum E122 { U1, U0, D1((Void, ())), D2((), Infallible), D0(Void), Live(ZPad<Three>) }
suite!(122, s122, E122, [E122::U1, E122::U0, E122::Live(ZPad([], Three::A, ())), E122::Live(ZPad([], Three::B, ())), E122::Live(ZPad([], Three::C, ()))], |e| match e { E122::U1 => 1002, E122::U0 => 1001, E122::D1(..) => 9002, E122::D2(..) => 9003, E122::D0(..) => 9001, E122::Live(w) => { let p: Three = w.1; p as u32 + 20 } });
#[derive(Clone, Copy)] enum E123 { D1(Wrap<Void>), U0, Live((Option<Option<Three>>, PhantomData<Void>, [Void; 0])), D0(()), U1 }
suite!(123, s123, E123, [E123::U0, E123::Live((Some(Some(Three::C)), PhantomData, [])), E123::Live((Some(None), PhantomData, [])), E123::Live((None, PhantomData, [])), E123::D0(()), E123::U1], |e| match e { E123::D1(..) => 9002, E123::U0 => 1001, E123::Live(w) => { let p: Option<Option<Three>> = w.0; match p { Some(Some(t)) => t as u32 + 60, Some(None) => 46, None => 45 } }, E123::D0(..) => 2001, E123::U1 => 1002 });
#[derive(Clone, Copy)] enum E124 { D1(([Void; 0], Void)), D0(Never), U1, Live(Result<W64, Void>), U2, U0 }
suite!(124, s124, E124, [E124::U1, E124::Live(Ok(W64::A)), E124::Live(Ok(W64::B)), E124::U2, E124::U0], |e| match e { E124::D1(..) => 9002, E124::D0(..) => 9001, E124::U1 => 1002, E124::Live(w) => { let p: W64 = (match w { Ok(x) => x, Err(_) => loop {} }); (p as u64 % 1_000_003) as u32 }, E124::U2 => 1003, E124::U0 => 1001 });
#[derive(Clone, Copy)] enum E125 { D1(&'static Void, Void), D0([u32; 0]), U0, D2([(Void,); 0]), Live(Result<Hi, Void>) }
suite!(125, s125, E125, [E125::D0([]), E125::U0, E125::D2([]), E125::Live(Ok(Hi::A)), E125::Live(Ok(Hi::B))], |e| match e { E125::D1(..) => 9002, E125::D0(..) => 2001, E125::U0 => 1001, E125::D2(..) => 2003, E125::Live(w) => { let p: Hi = (match w { Ok(x) => x, Err(_) => loop {} }); p as u32 } });
#[derive(Clone, Copy)] enum E126 { Live(Result<char, Void>), U0, D0(Void, [u64; 0]) }
suite!(126, s126, E126, [E126::Live(Ok('a')), E126::Live(Ok('\u{10FFFF}')), E126::Live(Ok('\0')), E126::U0], |e| match e { E126::Live(w) => { let p: char = (match w { Ok(x) => x, Err(_) => loop {} }); p as u32 }, E126::U0 => 1001, E126::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E127 { U1, Live(((), Hi2)), U0, D1(PhantomData<Void>), U2, D0(AllDead) }
suite!(127, s127, E127, [E127::U1, E127::Live(((), Hi2::A)), E127::Live(((), Hi2::B)), E127::U0, E127::D1(PhantomData), E127::U2], |e| match e { E127::U1 => 1002, E127::Live(w) => { let p: Hi2 = w.1; p as u32 }, E127::U0 => 1001, E127::D1(..) => 2002, E127::U2 => 1003, E127::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E128 { D0(NamedDead), Live((S32, PhantomData<Void>, [Void; 0])), U0 }
suite!(128, s128, E128, [E128::Live((S32::A, PhantomData, [])), E128::Live((S32::B, PhantomData, [])), E128::U0], |e| match e { E128::D0(..) => 9001, E128::Live(w) => { let p: S32 = w.0; p as i32 as u32 }, E128::U0 => 1001 });
#[derive(Clone, Copy)] enum E129 { Live(SL<Wrap<SL3<W32>>>), U0, D0(NamedDead) }
suite!(129, s129, E129, [E129::Live(SL::Live(Wrap(SL3::Live(W32::A)))), E129::Live(SL::Live(Wrap(SL3::Live(W32::B)))), E129::U0], |e| match e { E129::Live(w) => { let p: W32 = (match w { SL::Live(Wrap(SL3::Live(x))) => x, _ => loop {} }); p as u32 }, E129::U0 => 1001, E129::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E130 { D0(&'static Void, Void), D1((Never, Never)), Live(ZPad<Sg>), U0 }
suite!(130, s130, E130, [E130::Live(ZPad([], Sg::A, ())), E130::Live(ZPad([], Sg::B, ())), E130::U0], |e| match e { E130::D0(..) => 9001, E130::D1(..) => 9002, E130::Live(w) => { let p: Sg = w.1; p as i32 as u32 }, E130::U0 => 1001 });
#[derive(Clone, Copy)] enum E131 { D0(Void, (), PhantomData<u8>), Live(CW<S64>), D1(AllDead), U0, D2(AllDead) }
suite!(131, s131, E131, [E131::Live(CW(S64::A)), E131::Live(CW(S64::B)), E131::U0], |e| match e { E131::D0(..) => 9001, E131::Live(w) => { let p: S64 = w.0; (p as i64 as u64 % 1_000_003) as u32 }, E131::D1(..) => 9002, E131::U0 => 1001, E131::D2(..) => 9003 });
#[derive(Clone, Copy)] enum E132 { D0(u8, Void), Live(SL<Wrap<SL3<char>>>), D1(CDead) }
suite!(132, s132, E132, [E132::Live(SL::Live(Wrap(SL3::Live('a')))), E132::Live(SL::Live(Wrap(SL3::Live('\u{10FFFF}')))), E132::Live(SL::Live(Wrap(SL3::Live('\0'))))], |e| match e { E132::D0(..) => 9001, E132::Live(w) => { let p: char = (match w { SL::Live(Wrap(SL3::Live(x))) => x, _ => loop {} }); p as u32 }, E132::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E133 { Live(SL<Wrap<SL3<Sg>>>) }
suite!(133, s133, E133, [E133::Live(SL::Live(Wrap(SL3::Live(Sg::A)))), E133::Live(SL::Live(Wrap(SL3::Live(Sg::B))))], |e| match e { E133::Live(w) => { let p: Sg = (match w { SL::Live(Wrap(SL3::Live(x))) => x, _ => loop {} }); p as i32 as u32 } });
#[derive(Clone, Copy)] enum E134 { Live(Wrap<Hi3>), U0, U1 }
suite!(134, s134, E134, [E134::Live(Wrap(Hi3::A)), E134::Live(Wrap(Hi3::B)), E134::Live(Wrap(Hi3::C)), E134::U0, E134::U1], |e| match e { E134::Live(w) => { let p: Hi3 = w.0; p as u32 }, E134::U0 => 1001, E134::U1 => 1002 });
#[derive(Clone, Copy)] enum E135 { Live([W16; 1]) }
suite!(135, s135, E135, [E135::Live([W16::A]), E135::Live([W16::B])], |e| match e { E135::Live(w) => { let p: W16 = w[0]; p as u32 } });
#[derive(Clone, Copy)] enum E136 { D0(CDead), Live(SL3<Option<Option<Three>>>), U0 }
suite!(136, s136, E136, [E136::Live(SL3::Live(Some(Some(Three::C)))), E136::Live(SL3::Live(Some(None))), E136::Live(SL3::Live(None)), E136::U0], |e| match e { E136::D0(..) => 9001, E136::Live(w) => { let p: Option<Option<Three>> = (match w { SL3::Live(x) => x, _ => loop {} }); match p { Some(Some(t)) => t as u32 + 60, Some(None) => 46, None => 45 } }, E136::U0 => 1001 });
#[derive(Clone, Copy)] enum E137 { U0, U2, Live(Result<NonZeroU16, Void>), U4, U1, U3, U5 }
suite!(137, s137, E137, [E137::U0, E137::U2, E137::Live(Ok(NonZeroU16::new(1).unwrap())), E137::Live(Ok(NonZeroU16::new(65535).unwrap())), E137::U4, E137::U1, E137::U3, E137::U5], |e| match e { E137::U0 => 1001, E137::U2 => 1003, E137::Live(w) => { let p: NonZeroU16 = (match w { Ok(x) => x, Err(_) => loop {} }); p.get() as u32 }, E137::U4 => 1005, E137::U1 => 1002, E137::U3 => 1004, E137::U5 => 1006 });
#[derive(Clone, Copy)] enum E138 { Live(SL<NonZeroU8>), U0, D0((u32, Void)) }
suite!(138, s138, E138, [E138::Live(SL::Live(NonZeroU8::new(1).unwrap())), E138::Live(SL::Live(NonZeroU8::new(255).unwrap())), E138::U0], |e| match e { E138::Live(w) => { let p: NonZeroU8 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); p.get() as u32 }, E138::U0 => 1001, E138::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E139 { D2([u32; 0], Void), Live(Pk<Option<Hi2>>), D0(DeepDead), D1([(Void,); 0]) }
suite!(139, s139, E139, [E139::Live(Pk(Some(Hi2::A))), E139::Live(Pk(Some(Hi2::B))), E139::Live(Pk(None)), E139::D1([])], |e| match e { E139::D2(..) => 9003, E139::Live(w) => { let p: Option<Hi2> = { w.0 }; match p { Some(h) => h as u32, None => 47 } }, E139::D0(..) => 9001, E139::D1(..) => 2002 });

#[unsafe(no_mangle)]
pub fn rust_entry() {
    s120(); s121(); s122(); s123(); s124(); s125(); s126(); s127(); s128(); s129(); s130(); s131(); s132(); s133(); s134(); s135(); s136(); s137(); s138(); s139();
}
