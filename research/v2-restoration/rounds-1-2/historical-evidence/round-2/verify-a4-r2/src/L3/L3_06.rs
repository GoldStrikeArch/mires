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
#[derive(Clone, Copy)] enum E120 { D1(Aligned), U1, U0, Live(Pk<Two>), D0(PkDead) }
suite!(120, s120, E120, [E120::U1, E120::U0, E120::Live(Pk(Two::A)), E120::Live(Pk(Two::B))], [E120::Live(Pk(black_box(Two::A))), E120::Live(Pk(black_box(Two::B)))], |e| match e { E120::D1(..) => 9002, E120::U1 => 1002, E120::U0 => 1001, E120::Live(w) => { let p: Two = { w.0 }; p as u32 + 30 }, E120::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E121 { U0, U1, U3, Live(Wrap<NonZeroU16>), D0([Void; 0]), U2 }
suite!(121, s121, E121, [E121::U0, E121::U1, E121::U3, E121::Live(Wrap(NonZeroU16::new(1).unwrap())), E121::Live(Wrap(NonZeroU16::new(65535).unwrap())), E121::D0([]), E121::U2], [E121::Live(Wrap(black_box(NonZeroU16::new(1).unwrap()))), E121::Live(Wrap(black_box(NonZeroU16::new(65535).unwrap())))], |e| match e { E121::U0 => 1001, E121::U1 => 1002, E121::U3 => 1004, E121::Live(w) => { let p: NonZeroU16 = w.0; p.get() as u32 }, E121::D0(..) => 2001, E121::U2 => 1003 });
#[derive(Clone, Copy)] enum E122 { Live(((), Ordering)), D0(()), D1(HalfDead), D2(DeepDead), U2, U0, U1 }
suite!(122, s122, E122, [E122::Live(((), Ordering::Less)), E122::Live(((), Ordering::Equal)), E122::Live(((), Ordering::Greater)), E122::D0(()), E122::D1(HalfDead::B), E122::U2, E122::U0, E122::U1], [E122::Live(((), black_box(Ordering::Less))), E122::Live(((), black_box(Ordering::Equal))), E122::Live(((), black_box(Ordering::Greater)))], |e| match e { E122::Live(w) => { let p: Ordering = w.1; p as i32 as u32 }, E122::D0(..) => 2001, E122::D1(..) => 2002, E122::D2(..) => 9003, E122::U2 => 1003, E122::U0 => 1001, E122::U1 => 1002 });
#[derive(Clone, Copy)] enum E123 { Live(((), Hi3)), U1, U0 }
suite!(123, s123, E123, [E123::Live(((), Hi3::A)), E123::Live(((), Hi3::B)), E123::Live(((), Hi3::C)), E123::U1, E123::U0], [E123::Live(((), black_box(Hi3::A))), E123::Live(((), black_box(Hi3::B))), E123::Live(((), black_box(Hi3::C)))], |e| match e { E123::Live(w) => { let p: Hi3 = w.1; p as u32 }, E123::U1 => 1002, E123::U0 => 1001 });
#[derive(Clone, Copy)] enum E124 { D2(UDead), U1, D0(()), Live((Option<char>, PhantomData<Void>, [Void; 0])), D1([u32; 0], Void), U0 }
suite!(124, s124, E124, [E124::U1, E124::D0(()), E124::Live((Some('z'), PhantomData, [])), E124::Live((None, PhantomData, [])), E124::U0], [E124::Live((black_box(Some('z')), PhantomData, [])), E124::Live((black_box(None), PhantomData, []))], |e| match e { E124::D2(..) => 9003, E124::U1 => 1002, E124::D0(..) => 2001, E124::Live(w) => { let p: Option<char> = w.0; match p { Some(c) => c as u32, None => 48 } }, E124::D1(..) => 9002, E124::U0 => 1001 });
#[derive(Clone, Copy)] enum E125 { U0, Live(W32) }
suite!(125, s125, E125, [E125::U0, E125::Live(W32::A), E125::Live(W32::B)], [E125::Live(black_box(W32::A)), E125::Live(black_box(W32::B))], |e| match e { E125::U0 => 1001, E125::Live(w) => { let p: W32 = w; p as u32 } });
#[derive(Clone, Copy)] enum E126 { U1, D0(NamedDead), U0, Live(SL3<NonZeroU32>) }
suite!(126, s126, E126, [E126::U1, E126::U0, E126::Live(SL3::Live(NonZeroU32::new(1).unwrap())), E126::Live(SL3::Live(NonZeroU32::new(0xFFFF_FFFF).unwrap()))], [E126::Live(SL3::Live(black_box(NonZeroU32::new(1).unwrap()))), E126::Live(SL3::Live(black_box(NonZeroU32::new(0xFFFF_FFFF).unwrap())))], |e| match e { E126::U1 => 1002, E126::D0(..) => 9001, E126::U0 => 1001, E126::Live(w) => { let p: NonZeroU32 = (match w { SL3::Live(x) => x, _ => loop {} }); p.get() } });
#[derive(Clone, Copy)] enum E127 { U2, U1, U0, D0(TrDead), Live(ZPad<char>) }
suite!(127, s127, E127, [E127::U2, E127::U1, E127::U0, E127::Live(ZPad([], 'a', ())), E127::Live(ZPad([], '\u{10FFFF}', ())), E127::Live(ZPad([], '\0', ()))], [E127::Live(ZPad([], black_box('a'), ())), E127::Live(ZPad([], black_box('\u{10FFFF}'), ())), E127::Live(ZPad([], black_box('\0'), ()))], |e| match e { E127::U2 => 1003, E127::U1 => 1002, E127::U0 => 1001, E127::D0(..) => 9001, E127::Live(w) => { let p: char = w.1; p as u32 } });
#[derive(Clone, Copy)] enum E128 { D1(UDead), U2, U0, D0(Void, [u64; 0]), Live(Result<NonZeroU64, Void>), D2(Option<Void>), U1 }
suite!(128, s128, E128, [E128::U2, E128::U0, E128::Live(Ok(NonZeroU64::new(1).unwrap())), E128::Live(Ok(NonZeroU64::new(0xFFFF_FFFF_FFFF_FFFF).unwrap())), E128::Live(Ok(NonZeroU64::new(0x1_0000_0000).unwrap())), E128::D2(None), E128::U1], [E128::Live(Ok(black_box(NonZeroU64::new(1).unwrap()))), E128::Live(Ok(black_box(NonZeroU64::new(0xFFFF_FFFF_FFFF_FFFF).unwrap()))), E128::Live(Ok(black_box(NonZeroU64::new(0x1_0000_0000).unwrap())))], |e| match e { E128::D1(..) => 9002, E128::U2 => 1003, E128::U0 => 1001, E128::D0(..) => 9001, E128::Live(w) => { let p: NonZeroU64 = (match w { Ok(x) => x, Err(_) => loop {} }); (p.get() % 1_000_003) as u32 }, E128::D2(..) => 2003, E128::U1 => 1002 });
#[derive(Clone, Copy)] enum E129 { D1(Void, [u64; 0]), Live(Tr<NonZeroU64>), U2, U0, D0(Infallible), U1 }
suite!(129, s129, E129, [E129::Live(Tr(NonZeroU64::new(1).unwrap())), E129::Live(Tr(NonZeroU64::new(0xFFFF_FFFF_FFFF_FFFF).unwrap())), E129::Live(Tr(NonZeroU64::new(0x1_0000_0000).unwrap())), E129::U2, E129::U0, E129::U1], [E129::Live(Tr(black_box(NonZeroU64::new(1).unwrap()))), E129::Live(Tr(black_box(NonZeroU64::new(0xFFFF_FFFF_FFFF_FFFF).unwrap()))), E129::Live(Tr(black_box(NonZeroU64::new(0x1_0000_0000).unwrap())))], |e| match e { E129::D1(..) => 9002, E129::Live(w) => { let p: NonZeroU64 = w.0; (p.get() % 1_000_003) as u32 }, E129::U2 => 1003, E129::U0 => 1001, E129::D0(..) => 9001, E129::U1 => 1002 });
#[derive(Clone, Copy)] enum E130 { Live(Result<char, Void>), D1((Never, Never)), D0(()) }
suite!(130, s130, E130, [E130::Live(Ok('a')), E130::Live(Ok('\u{10FFFF}')), E130::Live(Ok('\0')), E130::D0(())], [E130::Live(Ok(black_box('a'))), E130::Live(Ok(black_box('\u{10FFFF}'))), E130::Live(Ok(black_box('\0')))], |e| match e { E130::Live(w) => { let p: char = (match w { Ok(x) => x, Err(_) => loop {} }); p as u32 }, E130::D1(..) => 9002, E130::D0(..) => 2001 });
#[derive(Clone, Copy)] enum E131 { U0, D0(HalfDead2), U1, Live(CW<S64>) }
suite!(131, s131, E131, [E131::U0, E131::D0(HalfDead2::A), E131::U1, E131::Live(CW(S64::A)), E131::Live(CW(S64::B))], [E131::Live(CW(black_box(S64::A))), E131::Live(CW(black_box(S64::B)))], |e| match e { E131::U0 => 1001, E131::D0(..) => 2001, E131::U1 => 1002, E131::Live(w) => { let p: S64 = w.0; (p as i64 as u64 % 1_000_003) as u32 } });
#[derive(Clone, Copy)] enum E132 { Live(SL<NonZeroI8>), D1(PkDead), U0, U1, D0(()) }
suite!(132, s132, E132, [E132::Live(SL::Live(NonZeroI8::new(-1).unwrap())), E132::Live(SL::Live(NonZeroI8::new(127).unwrap())), E132::U0, E132::U1, E132::D0(())], [E132::Live(SL::Live(black_box(NonZeroI8::new(-1).unwrap()))), E132::Live(SL::Live(black_box(NonZeroI8::new(127).unwrap())))], |e| match e { E132::Live(w) => { let p: NonZeroI8 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); p.get() as i32 as u32 }, E132::D1(..) => 9002, E132::U0 => 1001, E132::U1 => 1002, E132::D0(..) => 2001 });
#[derive(Clone, Copy)] enum E133 { D0(DeepDead), U1, U0, Live(Wrap<Tr<((), NonZeroU16)>>) }
suite!(133, s133, E133, [E133::U1, E133::U0, E133::Live(Wrap(Tr(((), NonZeroU16::new(1).unwrap())))), E133::Live(Wrap(Tr(((), NonZeroU16::new(65535).unwrap()))))], [E133::Live(Wrap(Tr(((), black_box(NonZeroU16::new(1).unwrap()))))), E133::Live(Wrap(Tr(((), black_box(NonZeroU16::new(65535).unwrap())))))], |e| match e { E133::D0(..) => 9001, E133::U1 => 1002, E133::U0 => 1001, E133::Live(w) => { let p: NonZeroU16 = w.0.0.1; p.get() as u32 } });
#[derive(Clone, Copy)] enum E134 { U1, Live(SL3<fn() -> u32>), U0 }
suite!(134, s134, E134, [E134::U1, E134::Live(SL3::Live(seven)), E134::Live(SL3::Live(eight)), E134::U0], [E134::Live(SL3::Live(black_box(seven))), E134::Live(SL3::Live(black_box(eight)))], |e| match e { E134::U1 => 1002, E134::Live(w) => { let p: fn() -> u32 = (match w { SL3::Live(x) => x, _ => loop {} }); p() }, E134::U0 => 1001 });
#[derive(Clone, Copy)] enum E135 { U0, D1((Never, Never)), D2(Infallible), D0(PhantomData<Void>), Live(Tr<W16>), U1 }
suite!(135, s135, E135, [E135::U0, E135::D0(PhantomData), E135::Live(Tr(W16::A)), E135::Live(Tr(W16::B)), E135::U1], [E135::Live(Tr(black_box(W16::A))), E135::Live(Tr(black_box(W16::B)))], |e| match e { E135::U0 => 1001, E135::D1(..) => 9002, E135::D2(..) => 9003, E135::D0(..) => 2001, E135::Live(w) => { let p: W16 = w.0; p as u32 }, E135::U1 => 1002 });
#[derive(Clone, Copy)] enum E136 { D0([u32; 0]), Live(SL3<W16>), U0 }
suite!(136, s136, E136, [E136::D0([]), E136::Live(SL3::Live(W16::A)), E136::Live(SL3::Live(W16::B)), E136::U0], [E136::Live(SL3::Live(black_box(W16::A))), E136::Live(SL3::Live(black_box(W16::B)))], |e| match e { E136::D0(..) => 2001, E136::Live(w) => { let p: W16 = (match w { SL3::Live(x) => x, _ => loop {} }); p as u32 }, E136::U0 => 1001 });
#[derive(Clone, Copy)] enum E137 { U1, Live([Sg; 1]), U0 }
suite!(137, s137, E137, [E137::U1, E137::Live([Sg::A]), E137::Live([Sg::B]), E137::U0], [E137::Live([black_box(Sg::A)]), E137::Live([black_box(Sg::B)])], |e| match e { E137::U1 => 1002, E137::Live(w) => { let p: Sg = w[0]; p as i32 as u32 }, E137::U0 => 1001 });
#[derive(Clone, Copy)] enum E138 { Live(Option<Option<Three>>), U0, U1, D0(Infallible) }
suite!(138, s138, E138, [E138::Live(Some(Some(Three::C))), E138::Live(Some(None)), E138::Live(None), E138::U0, E138::U1], [E138::Live(black_box(Some(Some(Three::C)))), E138::Live(black_box(Some(None))), E138::Live(black_box(None))], |e| match e { E138::Live(w) => { let p: Option<Option<Three>> = w; match p { Some(Some(t)) => t as u32 + 60, Some(None) => 46, None => 45 } }, E138::U0 => 1001, E138::U1 => 1002, E138::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E139 { U0, U1, D1(Wrap<Void>), D0(()), Live(SL<NonZeroU16>) }
suite!(139, s139, E139, [E139::U0, E139::U1, E139::D0(()), E139::Live(SL::Live(NonZeroU16::new(1).unwrap())), E139::Live(SL::Live(NonZeroU16::new(65535).unwrap()))], [E139::Live(SL::Live(black_box(NonZeroU16::new(1).unwrap()))), E139::Live(SL::Live(black_box(NonZeroU16::new(65535).unwrap())))], |e| match e { E139::U0 => 1001, E139::U1 => 1002, E139::D1(..) => 9002, E139::D0(..) => 2001, E139::Live(w) => { let p: NonZeroU16 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); p.get() as u32 } });

#[unsafe(no_mangle)]
pub fn rust_entry() {
    s120(); s121(); s122(); s123(); s124(); s125(); s126(); s127(); s128(); s129(); s130(); s131(); s132(); s133(); s134(); s135(); s136(); s137(); s138(); s139();
}
