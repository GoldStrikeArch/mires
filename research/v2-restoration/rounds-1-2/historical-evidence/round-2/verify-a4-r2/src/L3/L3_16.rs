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
#[derive(Clone, Copy)] enum E320 { U1, U0, D0(HalfDead2), Live(CW<Hi2>) }
suite!(320, s320, E320, [E320::U1, E320::U0, E320::D0(HalfDead2::A), E320::Live(CW(Hi2::A)), E320::Live(CW(Hi2::B))], [E320::Live(CW(black_box(Hi2::A))), E320::Live(CW(black_box(Hi2::B)))], |e| match e { E320::U1 => 1002, E320::U0 => 1001, E320::D0(..) => 2001, E320::Live(w) => { let p: Hi2 = w.0; p as u32 } });
#[derive(Clone, Copy)] enum E321 { U0, D1((Void, ())), D0([Void; 2]), U2, Live(SL<Wrap<SL3<Hi>>>), U3, U1 }
suite!(321, s321, E321, [E321::U0, E321::U2, E321::Live(SL::Live(Wrap(SL3::Live(Hi::A)))), E321::Live(SL::Live(Wrap(SL3::Live(Hi::B)))), E321::U3, E321::U1], [E321::Live(SL::Live(Wrap(SL3::Live(black_box(Hi::A))))), E321::Live(SL::Live(Wrap(SL3::Live(black_box(Hi::B)))))], |e| match e { E321::U0 => 1001, E321::D1(..) => 9002, E321::D0(..) => 9001, E321::U2 => 1003, E321::Live(w) => { let p: Hi = (match w { SL::Live(Wrap(SL3::Live(x))) => x, _ => loop {} }); p as u32 }, E321::U3 => 1004, E321::U1 => 1002 });
#[derive(Clone, Copy)] enum E322 { D0(u8, Void), Live(Wrap<Tr<((), Option<char>)>>), U0 }
suite!(322, s322, E322, [E322::Live(Wrap(Tr(((), Some('z'))))), E322::Live(Wrap(Tr(((), None)))), E322::U0], [E322::Live(Wrap(Tr(((), black_box(Some('z')))))), E322::Live(Wrap(Tr(((), black_box(None)))))], |e| match e { E322::D0(..) => 9001, E322::Live(w) => { let p: Option<char> = w.0.0.1; match p { Some(c) => c as u32, None => 48 } }, E322::U0 => 1001 });
#[derive(Clone, Copy)] enum E323 { Live(Wrap<Tr<((), NonZeroI16)>>), U3, U2, U1, U5, U0, U4 }
suite!(323, s323, E323, [E323::Live(Wrap(Tr(((), NonZeroI16::new(-1).unwrap())))), E323::Live(Wrap(Tr(((), NonZeroI16::new(300).unwrap())))), E323::U3, E323::U2, E323::U1, E323::U5, E323::U0, E323::U4], [E323::Live(Wrap(Tr(((), black_box(NonZeroI16::new(-1).unwrap()))))), E323::Live(Wrap(Tr(((), black_box(NonZeroI16::new(300).unwrap())))))], |e| match e { E323::Live(w) => { let p: NonZeroI16 = w.0.0.1; p.get() as i32 as u32 }, E323::U3 => 1004, E323::U2 => 1003, E323::U1 => 1002, E323::U5 => 1006, E323::U0 => 1001, E323::U4 => 1005 });
#[derive(Clone, Copy)] enum E324 { D1(NamedDead), Live(Named<&'static u32>), D2(CDead), D0(ManuallyDrop<Void>) }
suite!(324, s324, E324, [E324::Live(Named { z: (), v: &5, p: PhantomData }), E324::Live(Named { z: (), v: &SEVEN, p: PhantomData })], [E324::Live(Named { z: (), v: black_box(&5), p: PhantomData }), E324::Live(Named { z: (), v: black_box(&SEVEN), p: PhantomData })], |e| match e { E324::D1(..) => 9002, E324::Live(w) => { let p: &'static u32 = w.v; *p }, E324::D2(..) => 9003, E324::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E325 { Live((bool, PhantomData<Void>, [Void; 0])), U2, U0, D0(Option<Void>), U3, U1 }
suite!(325, s325, E325, [E325::Live((true, PhantomData, [])), E325::Live((false, PhantomData, [])), E325::U2, E325::U0, E325::D0(None), E325::U3, E325::U1], [E325::Live((black_box(true), PhantomData, [])), E325::Live((black_box(false), PhantomData, []))], |e| match e { E325::Live(w) => { let p: bool = w.0; 10 + p as u32 }, E325::U2 => 1003, E325::U0 => 1001, E325::D0(..) => 2001, E325::U3 => 1004, E325::U1 => 1002 });
#[derive(Clone, Copy)] enum E326 { Live(Wrap<Option<char>>), D1(DeepDead), D0(Option<Void>), U1, U0, D2([u16; 0], Void) }
suite!(326, s326, E326, [E326::Live(Wrap(Some('z'))), E326::Live(Wrap(None)), E326::D0(None), E326::U1, E326::U0], [E326::Live(Wrap(black_box(Some('z')))), E326::Live(Wrap(black_box(None)))], |e| match e { E326::Live(w) => { let p: Option<char> = w.0; match p { Some(c) => c as u32, None => 48 } }, E326::D1(..) => 9002, E326::D0(..) => 2001, E326::U1 => 1002, E326::U0 => 1001, E326::D2(..) => 9003 });
#[derive(Clone, Copy)] enum E327 { U1, Live(Pk<Hi>), U0 }
suite!(327, s327, E327, [E327::U1, E327::Live(Pk(Hi::A)), E327::Live(Pk(Hi::B)), E327::U0], [E327::Live(Pk(black_box(Hi::A))), E327::Live(Pk(black_box(Hi::B)))], |e| match e { E327::U1 => 1002, E327::Live(w) => { let p: Hi = { w.0 }; p as u32 }, E327::U0 => 1001 });
#[derive(Clone, Copy)] enum E328 { U0, Live(Result<S16, Void>), D0((Void,)) }
suite!(328, s328, E328, [E328::U0, E328::Live(Ok(S16::A)), E328::Live(Ok(S16::B))], [E328::Live(Ok(black_box(S16::A))), E328::Live(Ok(black_box(S16::B)))], |e| match e { E328::U0 => 1001, E328::Live(w) => { let p: S16 = (match w { Ok(x) => x, Err(_) => loop {} }); p as i32 as u32 }, E328::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E329 { U1, U0, Live(Hi2) }
suite!(329, s329, E329, [E329::U1, E329::U0, E329::Live(Hi2::A), E329::Live(Hi2::B)], [E329::Live(black_box(Hi2::A)), E329::Live(black_box(Hi2::B))], |e| match e { E329::U1 => 1002, E329::U0 => 1001, E329::Live(w) => { let p: Hi2 = w; p as u32 } });
#[derive(Clone, Copy)] enum E330 { U0, Live(ZPad<Hi3>), D0([u32; 0], Void), D1(Wrap<Wrap<(Void, ())>>) }
suite!(330, s330, E330, [E330::U0, E330::Live(ZPad([], Hi3::A, ())), E330::Live(ZPad([], Hi3::B, ())), E330::Live(ZPad([], Hi3::C, ()))], [E330::Live(ZPad([], black_box(Hi3::A), ())), E330::Live(ZPad([], black_box(Hi3::B), ())), E330::Live(ZPad([], black_box(Hi3::C), ()))], |e| match e { E330::U0 => 1001, E330::Live(w) => { let p: Hi3 = w.1; p as u32 }, E330::D0(..) => 9001, E330::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E331 { U0, U5, D0(TrDead), U4, Live((S16, PhantomData<Void>, [Void; 0])), D1(Option<Void>), U1, U3, U2 }
suite!(331, s331, E331, [E331::U0, E331::U5, E331::U4, E331::Live((S16::A, PhantomData, [])), E331::Live((S16::B, PhantomData, [])), E331::D1(None), E331::U1, E331::U3, E331::U2], [E331::Live((black_box(S16::A), PhantomData, [])), E331::Live((black_box(S16::B), PhantomData, []))], |e| match e { E331::U0 => 1001, E331::U5 => 1006, E331::D0(..) => 9001, E331::U4 => 1005, E331::Live(w) => { let p: S16 = w.0; p as i32 as u32 }, E331::D1(..) => 2002, E331::U1 => 1002, E331::U3 => 1004, E331::U2 => 1003 });
#[derive(Clone, Copy)] enum E332 { D1(TrDead), D0(Wrap<Wrap<(Void, ())>>), U0, Live(Wrap<Ordering>) }
suite!(332, s332, E332, [E332::U0, E332::Live(Wrap(Ordering::Less)), E332::Live(Wrap(Ordering::Equal)), E332::Live(Wrap(Ordering::Greater))], [E332::Live(Wrap(black_box(Ordering::Less))), E332::Live(Wrap(black_box(Ordering::Equal))), E332::Live(Wrap(black_box(Ordering::Greater)))], |e| match e { E332::D1(..) => 9002, E332::D0(..) => 9001, E332::U0 => 1001, E332::Live(w) => { let p: Ordering = w.0; p as i32 as u32 } });
#[derive(Clone, Copy)] enum E333 { U1, U0, D0(Wrap<Void>), D1(()), Live(Pk<Hi>) }
suite!(333, s333, E333, [E333::U1, E333::U0, E333::D1(()), E333::Live(Pk(Hi::A)), E333::Live(Pk(Hi::B))], [E333::Live(Pk(black_box(Hi::A))), E333::Live(Pk(black_box(Hi::B)))], |e| match e { E333::U1 => 1002, E333::U0 => 1001, E333::D0(..) => 9001, E333::D1(..) => 2002, E333::Live(w) => { let p: Hi = { w.0 }; p as u32 } });
#[derive(Clone, Copy)] enum E334 { D0([[Void; 1]; 1]), U0, Live(Tr<Option<Option<Three>>>), D1([(Void,); 0]) }
suite!(334, s334, E334, [E334::U0, E334::Live(Tr(Some(Some(Three::C)))), E334::Live(Tr(Some(None))), E334::Live(Tr(None)), E334::D1([])], [E334::Live(Tr(black_box(Some(Some(Three::C))))), E334::Live(Tr(black_box(Some(None)))), E334::Live(Tr(black_box(None)))], |e| match e { E334::D0(..) => 9001, E334::U0 => 1001, E334::Live(w) => { let p: Option<Option<Three>> = w.0; match p { Some(Some(t)) => t as u32 + 60, Some(None) => 46, None => 45 } }, E334::D1(..) => 2002 });
#[derive(Clone, Copy)] enum E335 { D0(PkDead), U0, U1, U2, Live(Sg2) }
suite!(335, s335, E335, [E335::U0, E335::U1, E335::U2, E335::Live(Sg2::A), E335::Live(Sg2::B)], [E335::Live(black_box(Sg2::A)), E335::Live(black_box(Sg2::B))], |e| match e { E335::D0(..) => 9001, E335::U0 => 1001, E335::U1 => 1002, E335::U2 => 1003, E335::Live(w) => { let p: Sg2 = w; p as i32 as u32 } });
#[derive(Clone, Copy)] enum E336 { U0, D0(Wrap<Void>), Live(ZPad<WU>) }
suite!(336, s336, E336, [E336::U0, E336::Live(ZPad([], WU::A, ())), E336::Live(ZPad([], WU::B, ()))], [E336::Live(ZPad([], black_box(WU::A), ())), E336::Live(ZPad([], black_box(WU::B), ()))], |e| match e { E336::U0 => 1001, E336::D0(..) => 9001, E336::Live(w) => { let p: WU = w.1; p as usize as u32 } });
#[derive(Clone, Copy)] enum E337 { D0([u16; 0], Void), D1(Wrap<AllDead>), U0, Live(Tr<NonZeroI16>) }
suite!(337, s337, E337, [E337::U0, E337::Live(Tr(NonZeroI16::new(-1).unwrap())), E337::Live(Tr(NonZeroI16::new(300).unwrap()))], [E337::Live(Tr(black_box(NonZeroI16::new(-1).unwrap()))), E337::Live(Tr(black_box(NonZeroI16::new(300).unwrap())))], |e| match e { E337::D0(..) => 9001, E337::D1(..) => 9002, E337::U0 => 1001, E337::Live(w) => { let p: NonZeroI16 = w.0; p.get() as i32 as u32 } });
#[derive(Clone, Copy)] enum E338 { D0(Never), U0, U2, U3, U1, Live(Result<Never, Option<Option<Three>>>) }
suite!(338, s338, E338, [E338::U0, E338::U2, E338::U3, E338::U1, E338::Live(Err(Some(Some(Three::C)))), E338::Live(Err(Some(None))), E338::Live(Err(None))], [E338::Live(Err(black_box(Some(Some(Three::C))))), E338::Live(Err(black_box(Some(None)))), E338::Live(Err(black_box(None)))], |e| match e { E338::D0(..) => 9001, E338::U0 => 1001, E338::U2 => 1003, E338::U3 => 1004, E338::U1 => 1002, E338::Live(w) => { let p: Option<Option<Three>> = (match w { Err(x) => x, Ok(_) => loop {} }); match p { Some(Some(t)) => t as u32 + 60, Some(None) => 46, None => 45 } } });
#[derive(Clone, Copy)] enum E339 { Live(Wrap<Tr<((), NonZeroU32)>>), U0, D0([u32; 0]), D1([(Void,); 0]) }
suite!(339, s339, E339, [E339::Live(Wrap(Tr(((), NonZeroU32::new(1).unwrap())))), E339::Live(Wrap(Tr(((), NonZeroU32::new(0xFFFF_FFFF).unwrap())))), E339::U0, E339::D0([]), E339::D1([])], [E339::Live(Wrap(Tr(((), black_box(NonZeroU32::new(1).unwrap()))))), E339::Live(Wrap(Tr(((), black_box(NonZeroU32::new(0xFFFF_FFFF).unwrap())))))], |e| match e { E339::Live(w) => { let p: NonZeroU32 = w.0.0.1; p.get() }, E339::U0 => 1001, E339::D0(..) => 2001, E339::D1(..) => 2002 });

#[unsafe(no_mangle)]
pub fn rust_entry() {
    s320(); s321(); s322(); s323(); s324(); s325(); s326(); s327(); s328(); s329(); s330(); s331(); s332(); s333(); s334(); s335(); s336(); s337(); s338(); s339();
}
