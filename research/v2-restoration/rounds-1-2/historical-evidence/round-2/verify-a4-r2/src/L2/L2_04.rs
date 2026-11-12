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
#[derive(Clone, Copy)] enum E80 { D0(AllDead), Live(Wrap<WU>), D1(Aligned) }
suite!(80, s80, E80, [E80::Live(Wrap(WU::A)), E80::Live(Wrap(WU::B))], |e| match e { E80::D0(..) => 9001, E80::Live(w) => { let p: WU = w.0; p as usize as u32 }, E80::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E81 { D2(TrDead), Live(Wrap<Tr<((), Sg)>>), D1(&'static Void, Void), D0(HalfDead) }
suite!(81, s81, E81, [E81::Live(Wrap(Tr(((), Sg::A)))), E81::Live(Wrap(Tr(((), Sg::B)))), E81::D0(HalfDead::B)], |e| match e { E81::D2(..) => 9003, E81::Live(w) => { let p: Sg = w.0.0.1; p as i32 as u32 }, E81::D1(..) => 9002, E81::D0(..) => 2001 });
#[derive(Clone, Copy)] enum E82 { U0, D1((Void, ())), Live(Named<bool>), D0(Void, (), PhantomData<u8>) }
suite!(82, s82, E82, [E82::U0, E82::Live(Named { z: (), v: true, p: PhantomData }), E82::Live(Named { z: (), v: false, p: PhantomData })], |e| match e { E82::U0 => 1001, E82::D1(..) => 9002, E82::Live(w) => { let p: bool = w.v; 10 + p as u32 }, E82::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E83 { Live(ManuallyDrop<S64>), U0 }
suite!(83, s83, E83, [E83::Live(ManuallyDrop::new(S64::A)), E83::Live(ManuallyDrop::new(S64::B)), E83::U0], |e| match e { E83::Live(w) => { let p: S64 = ManuallyDrop::into_inner(w); (p as i64 as u64 % 1_000_003) as u32 }, E83::U0 => 1001 });
#[derive(Clone, Copy)] enum E84 { Live(((), S64)), D0(()), U0 }
suite!(84, s84, E84, [E84::Live(((), S64::A)), E84::Live(((), S64::B)), E84::D0(()), E84::U0], |e| match e { E84::Live(w) => { let p: S64 = w.1; (p as i64 as u64 % 1_000_003) as u32 }, E84::D0(..) => 2001, E84::U0 => 1001 });
#[derive(Clone, Copy)] enum E85 { U1, D0(UDead), U0, U2, Live(S32), D1(CDead) }
suite!(85, s85, E85, [E85::U1, E85::U0, E85::U2, E85::Live(S32::A), E85::Live(S32::B)], |e| match e { E85::U1 => 1002, E85::D0(..) => 9001, E85::U0 => 1001, E85::U2 => 1003, E85::Live(w) => { let p: S32 = w; p as i32 as u32 }, E85::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E86 { Live(Option<Option<Three>>), D1(Result<(), Void>), U0, D0(()) }
suite!(86, s86, E86, [E86::Live(Some(Some(Three::C))), E86::Live(Some(None)), E86::Live(None), E86::D1(Ok(())), E86::U0, E86::D0(())], |e| match e { E86::Live(w) => { let p: Option<Option<Three>> = w; match p { Some(Some(t)) => t as u32 + 60, Some(None) => 46, None => 45 } }, E86::D1(..) => 2002, E86::U0 => 1001, E86::D0(..) => 2001 });
#[derive(Clone, Copy)] enum E87 { Live(Pk<Two>), U1, D0(Result<Void, Never>), U2, U0 }
suite!(87, s87, E87, [E87::Live(Pk(Two::A)), E87::Live(Pk(Two::B)), E87::U1, E87::U2, E87::U0], |e| match e { E87::Live(w) => { let p: Two = { w.0 }; p as u32 + 30 }, E87::U1 => 1002, E87::D0(..) => 9001, E87::U2 => 1003, E87::U0 => 1001 });
#[derive(Clone, Copy)] enum E88 { U1, Live(SL<Wrap<SL3<W16>>>), D1(&'static Void, Void), D0(Option<Void>), U0 }
suite!(88, s88, E88, [E88::U1, E88::Live(SL::Live(Wrap(SL3::Live(W16::A)))), E88::Live(SL::Live(Wrap(SL3::Live(W16::B)))), E88::D0(None), E88::U0], |e| match e { E88::U1 => 1002, E88::Live(w) => { let p: W16 = (match w { SL::Live(Wrap(SL3::Live(x))) => x, _ => loop {} }); p as u32 }, E88::D1(..) => 9002, E88::D0(..) => 2001, E88::U0 => 1001 });
#[derive(Clone, Copy)] enum E89 { Live(((), Hi2)), D0(MaybeUninit<Void>) }
suite!(89, s89, E89, [E89::Live(((), Hi2::A)), E89::Live(((), Hi2::B))], |e| match e { E89::Live(w) => { let p: Hi2 = w.1; p as u32 }, E89::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E90 { Live(Result<S64, Void>), D1(AllDead), D0(HalfDead), U0 }
suite!(90, s90, E90, [E90::Live(Ok(S64::A)), E90::Live(Ok(S64::B)), E90::D0(HalfDead::B), E90::U0], |e| match e { E90::Live(w) => { let p: S64 = (match w { Ok(x) => x, Err(_) => loop {} }); (p as i64 as u64 % 1_000_003) as u32 }, E90::D1(..) => 9002, E90::D0(..) => 2001, E90::U0 => 1001 });
#[derive(Clone, Copy)] enum E91 { Live(((), fn() -> u32)) }
suite!(91, s91, E91, [E91::Live(((), seven)), E91::Live(((), eight))], |e| match e { E91::Live(w) => { let p: fn() -> u32 = w.1; p() } });
#[derive(Clone, Copy)] enum E92 { D0(HalfDead), U0, Live(&'static u32) }
suite!(92, s92, E92, [E92::D0(HalfDead::B), E92::U0, E92::Live(&5), E92::Live(&SEVEN)], |e| match e { E92::D0(..) => 2001, E92::U0 => 1001, E92::Live(w) => { let p: &'static u32 = w; *p } });
#[derive(Clone, Copy)] enum E93 { U0, Live((Small,)), U1, U3, U2 }
suite!(93, s93, E93, [E93::U0, E93::Live((Small::X,)), E93::Live((Small::Y,)), E93::U1, E93::U3, E93::U2], |e| match e { E93::U0 => 1001, E93::Live(w) => { let p: Small = w.0; p as u32 }, E93::U1 => 1002, E93::U3 => 1004, E93::U2 => 1003 });
#[derive(Clone, Copy)] enum E94 { U0, D0(MaybeUninit<Void>), Live(Wrap<Tr<((), Hi)>>) }
suite!(94, s94, E94, [E94::U0, E94::Live(Wrap(Tr(((), Hi::A)))), E94::Live(Wrap(Tr(((), Hi::B))))], |e| match e { E94::U0 => 1001, E94::D0(..) => 9001, E94::Live(w) => { let p: Hi = w.0.0.1; p as u32 } });
#[derive(Clone, Copy)] enum E95 { D1([Void; 0]), Live([Ordering; 1]), D0(&'static Void, Void), D2(NamedDead) }
suite!(95, s95, E95, [E95::D1([]), E95::Live([Ordering::Less]), E95::Live([Ordering::Equal]), E95::Live([Ordering::Greater])], |e| match e { E95::D1(..) => 2002, E95::Live(w) => { let p: Ordering = w[0]; p as i32 as u32 }, E95::D0(..) => 9001, E95::D2(..) => 9003 });
#[derive(Clone, Copy)] enum E96 { Live(Wrap<Tr<((), NonZeroU16)>>), D0(Result<(), Void>), U1, U0 }
suite!(96, s96, E96, [E96::Live(Wrap(Tr(((), NonZeroU16::new(1).unwrap())))), E96::Live(Wrap(Tr(((), NonZeroU16::new(65535).unwrap())))), E96::D0(Ok(())), E96::U1, E96::U0], |e| match e { E96::Live(w) => { let p: NonZeroU16 = w.0.0.1; p.get() as u32 }, E96::D0(..) => 2001, E96::U1 => 1002, E96::U0 => 1001 });
#[derive(Clone, Copy)] enum E97 { U5, D0((HalfDead, Void)), U4, U0, U1, Live([W64; 1]), U3, D1((), Infallible), U2 }
suite!(97, s97, E97, [E97::U5, E97::U4, E97::U0, E97::U1, E97::Live([W64::A]), E97::Live([W64::B]), E97::U3, E97::U2], |e| match e { E97::U5 => 1006, E97::D0(..) => 9001, E97::U4 => 1005, E97::U0 => 1001, E97::U1 => 1002, E97::Live(w) => { let p: W64 = w[0]; (p as u64 % 1_000_003) as u32 }, E97::U3 => 1004, E97::D1(..) => 9002, E97::U2 => 1003 });
#[derive(Clone, Copy)] enum E98 { D1((Void,)), D0(HalfDead), Live([Sg3; 1]), U0, U1, U2 }
suite!(98, s98, E98, [E98::D0(HalfDead::B), E98::Live([Sg3::A]), E98::Live([Sg3::B]), E98::U0, E98::U1, E98::U2], |e| match e { E98::D1(..) => 9002, E98::D0(..) => 2001, E98::Live(w) => { let p: Sg3 = w[0]; p as i32 as u32 }, E98::U0 => 1001, E98::U1 => 1002, E98::U2 => 1003 });
#[derive(Clone, Copy)] enum E99 { D1((Void, ())), U3, U1, U0, D0((Never, Never)), U2, U4, U5, D2(PhantomData<Void>), Live(S16) }
suite!(99, s99, E99, [E99::U3, E99::U1, E99::U0, E99::U2, E99::U4, E99::U5, E99::D2(PhantomData), E99::Live(S16::A), E99::Live(S16::B)], |e| match e { E99::D1(..) => 9002, E99::U3 => 1004, E99::U1 => 1002, E99::U0 => 1001, E99::D0(..) => 9001, E99::U2 => 1003, E99::U4 => 1005, E99::U5 => 1006, E99::D2(..) => 2003, E99::Live(w) => { let p: S16 = w; p as i32 as u32 } });

#[unsafe(no_mangle)]
pub fn rust_entry() {
    s80(); s81(); s82(); s83(); s84(); s85(); s86(); s87(); s88(); s89(); s90(); s91(); s92(); s93(); s94(); s95(); s96(); s97(); s98(); s99();
}
