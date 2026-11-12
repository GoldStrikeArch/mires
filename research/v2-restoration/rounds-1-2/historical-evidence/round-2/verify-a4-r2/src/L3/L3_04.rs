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
#[derive(Clone, Copy)] enum E80 { D0((Never, Never)), D1([(Void,); 0]), Live(SL3<NonZeroU16>) }
suite!(80, s80, E80, [E80::D1([]), E80::Live(SL3::Live(NonZeroU16::new(1).unwrap())), E80::Live(SL3::Live(NonZeroU16::new(65535).unwrap()))], [E80::Live(SL3::Live(black_box(NonZeroU16::new(1).unwrap()))), E80::Live(SL3::Live(black_box(NonZeroU16::new(65535).unwrap())))], |e| match e { E80::D0(..) => 9001, E80::D1(..) => 2002, E80::Live(w) => { let p: NonZeroU16 = (match w { SL3::Live(x) => x, _ => loop {} }); p.get() as u32 } });
#[derive(Clone, Copy)] enum E81 { D0([[Void; 1]; 1]), Live(Result<Never, NonZeroU64>), D1(Wrap<Wrap<(Void, ())>>) }
suite!(81, s81, E81, [E81::Live(Err(NonZeroU64::new(1).unwrap())), E81::Live(Err(NonZeroU64::new(0xFFFF_FFFF_FFFF_FFFF).unwrap())), E81::Live(Err(NonZeroU64::new(0x1_0000_0000).unwrap()))], [E81::Live(Err(black_box(NonZeroU64::new(1).unwrap()))), E81::Live(Err(black_box(NonZeroU64::new(0xFFFF_FFFF_FFFF_FFFF).unwrap()))), E81::Live(Err(black_box(NonZeroU64::new(0x1_0000_0000).unwrap())))], |e| match e { E81::D0(..) => 9001, E81::Live(w) => { let p: NonZeroU64 = (match w { Err(x) => x, Ok(_) => loop {} }); (p.get() % 1_000_003) as u32 }, E81::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E82 { U0, Live((Hi2, PhantomData<Void>, [Void; 0])) }
suite!(82, s82, E82, [E82::U0, E82::Live((Hi2::A, PhantomData, [])), E82::Live((Hi2::B, PhantomData, []))], [E82::Live((black_box(Hi2::A), PhantomData, [])), E82::Live((black_box(Hi2::B), PhantomData, []))], |e| match e { E82::U0 => 1001, E82::Live(w) => { let p: Hi2 = w.0; p as u32 } });
#[derive(Clone, Copy)] enum E83 { U1, U0, D0(DeepDead), Live(Three) }
suite!(83, s83, E83, [E83::U1, E83::U0, E83::Live(Three::A), E83::Live(Three::B), E83::Live(Three::C)], [E83::Live(black_box(Three::A)), E83::Live(black_box(Three::B)), E83::Live(black_box(Three::C))], |e| match e { E83::U1 => 1002, E83::U0 => 1001, E83::D0(..) => 9001, E83::Live(w) => { let p: Three = w; p as u32 + 20 } });
#[derive(Clone, Copy)] enum E84 { Live((W16,)) }
suite!(84, s84, E84, [E84::Live((W16::A,)), E84::Live((W16::B,))], [E84::Live((black_box(W16::A),)), E84::Live((black_box(W16::B),))], |e| match e { E84::Live(w) => { let p: W16 = w.0; p as u32 } });
#[derive(Clone, Copy)] enum E85 { Live(Sg2), D0([Void; 2]), D2(HalfDead), D1((HalfDead, Void)), U0 }
suite!(85, s85, E85, [E85::Live(Sg2::A), E85::Live(Sg2::B), E85::D2(HalfDead::B), E85::U0], [E85::Live(black_box(Sg2::A)), E85::Live(black_box(Sg2::B))], |e| match e { E85::Live(w) => { let p: Sg2 = w; p as i32 as u32 }, E85::D0(..) => 9001, E85::D2(..) => 2003, E85::D1(..) => 9002, E85::U0 => 1001 });
#[derive(Clone, Copy)] enum E86 { Live(Result<NonZeroU32, Void>), U1, U0 }
suite!(86, s86, E86, [E86::Live(Ok(NonZeroU32::new(1).unwrap())), E86::Live(Ok(NonZeroU32::new(0xFFFF_FFFF).unwrap())), E86::U1, E86::U0], [E86::Live(Ok(black_box(NonZeroU32::new(1).unwrap()))), E86::Live(Ok(black_box(NonZeroU32::new(0xFFFF_FFFF).unwrap())))], |e| match e { E86::Live(w) => { let p: NonZeroU32 = (match w { Ok(x) => x, Err(_) => loop {} }); p.get() }, E86::U1 => 1002, E86::U0 => 1001 });
#[derive(Clone, Copy)] enum E87 { U3, D0([u16; 0], Void), U0, U1, Live([Ordering; 1]), U2 }
suite!(87, s87, E87, [E87::U3, E87::U0, E87::U1, E87::Live([Ordering::Less]), E87::Live([Ordering::Equal]), E87::Live([Ordering::Greater]), E87::U2], [E87::Live([black_box(Ordering::Less)]), E87::Live([black_box(Ordering::Equal)]), E87::Live([black_box(Ordering::Greater)])], |e| match e { E87::U3 => 1004, E87::D0(..) => 9001, E87::U0 => 1001, E87::U1 => 1002, E87::Live(w) => { let p: Ordering = w[0]; p as i32 as u32 }, E87::U2 => 1003 });
#[derive(Clone, Copy)] enum E88 { D0([u32; 0], Void), U1, U2, U0, Live((S16, PhantomData<Void>, [Void; 0])) }
suite!(88, s88, E88, [E88::U1, E88::U2, E88::U0, E88::Live((S16::A, PhantomData, [])), E88::Live((S16::B, PhantomData, []))], [E88::Live((black_box(S16::A), PhantomData, [])), E88::Live((black_box(S16::B), PhantomData, []))], |e| match e { E88::D0(..) => 9001, E88::U1 => 1002, E88::U2 => 1003, E88::U0 => 1001, E88::Live(w) => { let p: S16 = w.0; p as i32 as u32 } });
#[derive(Clone, Copy)] enum E89 { U3, U4, D0(UDead), U2, U5, Live(SL3<Option<bool>>), U0, U1 }
suite!(89, s89, E89, [E89::U3, E89::U4, E89::U2, E89::U5, E89::Live(SL3::Live(Some(true))), E89::Live(SL3::Live(Some(false))), E89::Live(SL3::Live(None)), E89::U0, E89::U1], [E89::Live(SL3::Live(black_box(Some(true)))), E89::Live(SL3::Live(black_box(Some(false)))), E89::Live(SL3::Live(black_box(None)))], |e| match e { E89::U3 => 1004, E89::U4 => 1005, E89::D0(..) => 9001, E89::U2 => 1003, E89::U5 => 1006, E89::Live(w) => { let p: Option<bool> = (match w { SL3::Live(x) => x, _ => loop {} }); match p { Some(b) => 40 + b as u32, None => 49 } }, E89::U0 => 1001, E89::U1 => 1002 });
#[derive(Clone, Copy)] enum E90 { D1([Void; 0]), D0(u8, Void), Live(Tr<char>) }
suite!(90, s90, E90, [E90::D1([]), E90::Live(Tr('a')), E90::Live(Tr('\u{10FFFF}')), E90::Live(Tr('\0'))], [E90::Live(Tr(black_box('a'))), E90::Live(Tr(black_box('\u{10FFFF}'))), E90::Live(Tr(black_box('\0')))], |e| match e { E90::D1(..) => 2002, E90::D0(..) => 9001, E90::Live(w) => { let p: char = w.0; p as u32 } });
#[derive(Clone, Copy)] enum E91 { U0, D0(NamedDead), D1([(Void,); 3]), Live(bool), U2, U1 }
suite!(91, s91, E91, [E91::U0, E91::Live(true), E91::Live(false), E91::U2, E91::U1], [E91::Live(black_box(true)), E91::Live(black_box(false))], |e| match e { E91::U0 => 1001, E91::D0(..) => 9001, E91::D1(..) => 9002, E91::Live(w) => { let p: bool = w; 10 + p as u32 }, E91::U2 => 1003, E91::U1 => 1002 });
#[derive(Clone, Copy)] enum E92 { U1, U2, Live([bool; 1]), U3, U0 }
suite!(92, s92, E92, [E92::U1, E92::U2, E92::Live([true]), E92::Live([false]), E92::U3, E92::U0], [E92::Live([black_box(true)]), E92::Live([black_box(false)])], |e| match e { E92::U1 => 1002, E92::U2 => 1003, E92::Live(w) => { let p: bool = w[0]; 10 + p as u32 }, E92::U3 => 1004, E92::U0 => 1001 });
#[derive(Clone, Copy)] enum E93 { U0, Live(Pk<W16>), D0(([Void; 0], Void)) }
suite!(93, s93, E93, [E93::U0, E93::Live(Pk(W16::A)), E93::Live(Pk(W16::B))], [E93::Live(Pk(black_box(W16::A))), E93::Live(Pk(black_box(W16::B)))], |e| match e { E93::U0 => 1001, E93::Live(w) => { let p: W16 = { w.0 }; p as u32 }, E93::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E94 { Live(Wrap<Option<Hi2>>), D2(ManuallyDrop<Void>), D0([Void; 0]), D1(NamedDead) }
suite!(94, s94, E94, [E94::Live(Wrap(Some(Hi2::A))), E94::Live(Wrap(Some(Hi2::B))), E94::Live(Wrap(None)), E94::D0([])], [E94::Live(Wrap(black_box(Some(Hi2::A)))), E94::Live(Wrap(black_box(Some(Hi2::B)))), E94::Live(Wrap(black_box(None)))], |e| match e { E94::Live(w) => { let p: Option<Hi2> = w.0; match p { Some(h) => h as u32, None => 47 } }, E94::D2(..) => 9003, E94::D0(..) => 2001, E94::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E95 { Live(ZPad<W32>), D0(Wrap<Wrap<(Void, ())>>) }
suite!(95, s95, E95, [E95::Live(ZPad([], W32::A, ())), E95::Live(ZPad([], W32::B, ()))], [E95::Live(ZPad([], black_box(W32::A), ())), E95::Live(ZPad([], black_box(W32::B), ()))], |e| match e { E95::Live(w) => { let p: W32 = w.1; p as u32 }, E95::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E96 { U0, U1, U2, Live(NonZeroU8) }
suite!(96, s96, E96, [E96::U0, E96::U1, E96::U2, E96::Live(NonZeroU8::new(1).unwrap()), E96::Live(NonZeroU8::new(255).unwrap())], [E96::Live(black_box(NonZeroU8::new(1).unwrap())), E96::Live(black_box(NonZeroU8::new(255).unwrap()))], |e| match e { E96::U0 => 1001, E96::U1 => 1002, E96::U2 => 1003, E96::Live(w) => { let p: NonZeroU8 = w; p.get() as u32 } });
#[derive(Clone, Copy)] enum E97 { D0((Void,)), Live(WU) }
suite!(97, s97, E97, [E97::Live(WU::A), E97::Live(WU::B)], [E97::Live(black_box(WU::A)), E97::Live(black_box(WU::B))], |e| match e { E97::D0(..) => 9001, E97::Live(w) => { let p: WU = w; p as usize as u32 } });
#[derive(Clone, Copy)] enum E98 { Live(Result<Never, NonZeroU32>), U1, U0, D0(([Void; 0], Void)) }
suite!(98, s98, E98, [E98::Live(Err(NonZeroU32::new(1).unwrap())), E98::Live(Err(NonZeroU32::new(0xFFFF_FFFF).unwrap())), E98::U1, E98::U0], [E98::Live(Err(black_box(NonZeroU32::new(1).unwrap()))), E98::Live(Err(black_box(NonZeroU32::new(0xFFFF_FFFF).unwrap())))], |e| match e { E98::Live(w) => { let p: NonZeroU32 = (match w { Err(x) => x, Ok(_) => loop {} }); p.get() }, E98::U1 => 1002, E98::U0 => 1001, E98::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E99 { U1, U0, Live(Option<Option<Three>>), U2, D0(u8, Void) }
suite!(99, s99, E99, [E99::U1, E99::U0, E99::Live(Some(Some(Three::C))), E99::Live(Some(None)), E99::Live(None), E99::U2], [E99::Live(black_box(Some(Some(Three::C)))), E99::Live(black_box(Some(None))), E99::Live(black_box(None))], |e| match e { E99::U1 => 1002, E99::U0 => 1001, E99::Live(w) => { let p: Option<Option<Three>> = w; match p { Some(Some(t)) => t as u32 + 60, Some(None) => 46, None => 45 } }, E99::U2 => 1003, E99::D0(..) => 9001 });

#[unsafe(no_mangle)]
pub fn rust_entry() {
    s80(); s81(); s82(); s83(); s84(); s85(); s86(); s87(); s88(); s89(); s90(); s91(); s92(); s93(); s94(); s95(); s96(); s97(); s98(); s99();
}
