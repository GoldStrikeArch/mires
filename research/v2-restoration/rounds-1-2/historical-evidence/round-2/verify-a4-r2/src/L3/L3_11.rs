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
#[derive(Clone, Copy)] enum E220 { Live((Sg3,)), D0(UDead) }
suite!(220, s220, E220, [E220::Live((Sg3::A,)), E220::Live((Sg3::B,))], [E220::Live((black_box(Sg3::A),)), E220::Live((black_box(Sg3::B),))], |e| match e { E220::Live(w) => { let p: Sg3 = w.0; p as i32 as u32 }, E220::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E221 { D0(&'static Void, Void), U0, Live(Result<Small, Void>), U1 }
suite!(221, s221, E221, [E221::U0, E221::Live(Ok(Small::X)), E221::Live(Ok(Small::Y)), E221::U1], [E221::Live(Ok(black_box(Small::X))), E221::Live(Ok(black_box(Small::Y)))], |e| match e { E221::D0(..) => 9001, E221::U0 => 1001, E221::Live(w) => { let p: Small = (match w { Ok(x) => x, Err(_) => loop {} }); p as u32 }, E221::U1 => 1002 });
#[derive(Clone, Copy)] enum E222 { U0, U2, U3, Live(SL<Option<Option<Three>>>), U1, D0(Result<(), Void>) }
suite!(222, s222, E222, [E222::U0, E222::U2, E222::U3, E222::Live(SL::Live(Some(Some(Three::C)))), E222::Live(SL::Live(Some(None))), E222::Live(SL::Live(None)), E222::U1, E222::D0(Ok(()))], [E222::Live(SL::Live(black_box(Some(Some(Three::C))))), E222::Live(SL::Live(black_box(Some(None)))), E222::Live(SL::Live(black_box(None)))], |e| match e { E222::U0 => 1001, E222::U2 => 1003, E222::U3 => 1004, E222::Live(w) => { let p: Option<Option<Three>> = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); match p { Some(Some(t)) => t as u32 + 60, Some(None) => 46, None => 45 } }, E222::U1 => 1002, E222::D0(..) => 2001 });
#[derive(Clone, Copy)] enum E223 { U1, Live(SL<Wrap<SL3<bool>>>), D0([u32; 0]), U0, U2 }
suite!(223, s223, E223, [E223::U1, E223::Live(SL::Live(Wrap(SL3::Live(true)))), E223::Live(SL::Live(Wrap(SL3::Live(false)))), E223::D0([]), E223::U0, E223::U2], [E223::Live(SL::Live(Wrap(SL3::Live(black_box(true))))), E223::Live(SL::Live(Wrap(SL3::Live(black_box(false)))))], |e| match e { E223::U1 => 1002, E223::Live(w) => { let p: bool = (match w { SL::Live(Wrap(SL3::Live(x))) => x, _ => loop {} }); 10 + p as u32 }, E223::D0(..) => 2001, E223::U0 => 1001, E223::U2 => 1003 });
#[derive(Clone, Copy)] enum E224 { Live(Result<Never, NonZeroI16>), U3, U0, U1, D0(Infallible), U2 }
suite!(224, s224, E224, [E224::Live(Err(NonZeroI16::new(-1).unwrap())), E224::Live(Err(NonZeroI16::new(300).unwrap())), E224::U3, E224::U0, E224::U1, E224::U2], [E224::Live(Err(black_box(NonZeroI16::new(-1).unwrap()))), E224::Live(Err(black_box(NonZeroI16::new(300).unwrap())))], |e| match e { E224::Live(w) => { let p: NonZeroI16 = (match w { Err(x) => x, Ok(_) => loop {} }); p.get() as i32 as u32 }, E224::U3 => 1004, E224::U0 => 1001, E224::U1 => 1002, E224::D0(..) => 9001, E224::U2 => 1003 });
#[derive(Clone, Copy)] enum E225 { D2(Aligned), D1(MaybeUninit<Void>), Live([Hi3; 1]), U0, U1, D0(Never) }
suite!(225, s225, E225, [E225::Live([Hi3::A]), E225::Live([Hi3::B]), E225::Live([Hi3::C]), E225::U0, E225::U1], [E225::Live([black_box(Hi3::A)]), E225::Live([black_box(Hi3::B)]), E225::Live([black_box(Hi3::C)])], |e| match e { E225::D2(..) => 9003, E225::D1(..) => 9002, E225::Live(w) => { let p: Hi3 = w[0]; p as u32 }, E225::U0 => 1001, E225::U1 => 1002, E225::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E226 { Live(SL<S64>), D1(&'static Void, Void), D0(Result<Void, Never>) }
suite!(226, s226, E226, [E226::Live(SL::Live(S64::A)), E226::Live(SL::Live(S64::B))], [E226::Live(SL::Live(black_box(S64::A))), E226::Live(SL::Live(black_box(S64::B)))], |e| match e { E226::Live(w) => { let p: S64 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); (p as i64 as u64 % 1_000_003) as u32 }, E226::D1(..) => 9002, E226::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E227 { U5, U1, U4, U0, U3, Live(Result<Never, S16>), D1(Option<Void>), D0((Void, ())), U2 }
suite!(227, s227, E227, [E227::U5, E227::U1, E227::U4, E227::U0, E227::U3, E227::Live(Err(S16::A)), E227::Live(Err(S16::B)), E227::D1(None), E227::U2], [E227::Live(Err(black_box(S16::A))), E227::Live(Err(black_box(S16::B)))], |e| match e { E227::U5 => 1006, E227::U1 => 1002, E227::U4 => 1005, E227::U0 => 1001, E227::U3 => 1004, E227::Live(w) => { let p: S16 = (match w { Err(x) => x, Ok(_) => loop {} }); p as i32 as u32 }, E227::D1(..) => 2002, E227::D0(..) => 9001, E227::U2 => 1003 });
#[derive(Clone, Copy)] enum E228 { U0, U2, U1, D0(PkDead), D1(HalfDead2), Live(Tr<fn() -> u32>) }
suite!(228, s228, E228, [E228::U0, E228::U2, E228::U1, E228::D1(HalfDead2::A), E228::Live(Tr(seven)), E228::Live(Tr(eight))], [E228::Live(Tr(black_box(seven))), E228::Live(Tr(black_box(eight)))], |e| match e { E228::U0 => 1001, E228::U2 => 1003, E228::U1 => 1002, E228::D0(..) => 9001, E228::D1(..) => 2002, E228::Live(w) => { let p: fn() -> u32 = w.0; p() } });
#[derive(Clone, Copy)] enum E229 { Live(Tr<Option<bool>>), U2, U1, D0([Void; 0]), D1(Wrap<Wrap<(Void, ())>>), U0 }
suite!(229, s229, E229, [E229::Live(Tr(Some(true))), E229::Live(Tr(Some(false))), E229::Live(Tr(None)), E229::U2, E229::U1, E229::D0([]), E229::U0], [E229::Live(Tr(black_box(Some(true)))), E229::Live(Tr(black_box(Some(false)))), E229::Live(Tr(black_box(None)))], |e| match e { E229::Live(w) => { let p: Option<bool> = w.0; match p { Some(b) => 40 + b as u32, None => 49 } }, E229::U2 => 1003, E229::U1 => 1002, E229::D0(..) => 2001, E229::D1(..) => 9002, E229::U0 => 1001 });
#[derive(Clone, Copy)] enum E230 { U4, U2, U0, U5, Live(SL3<S64>), U3, U1 }
suite!(230, s230, E230, [E230::U4, E230::U2, E230::U0, E230::U5, E230::Live(SL3::Live(S64::A)), E230::Live(SL3::Live(S64::B)), E230::U3, E230::U1], [E230::Live(SL3::Live(black_box(S64::A))), E230::Live(SL3::Live(black_box(S64::B)))], |e| match e { E230::U4 => 1005, E230::U2 => 1003, E230::U0 => 1001, E230::U5 => 1006, E230::Live(w) => { let p: S64 = (match w { SL3::Live(x) => x, _ => loop {} }); (p as i64 as u64 % 1_000_003) as u32 }, E230::U3 => 1004, E230::U1 => 1002 });
#[derive(Clone, Copy)] enum E231 { U1, U0, D0(NamedDead), Live(Wrap<NonZeroU32>) }
suite!(231, s231, E231, [E231::U1, E231::U0, E231::Live(Wrap(NonZeroU32::new(1).unwrap())), E231::Live(Wrap(NonZeroU32::new(0xFFFF_FFFF).unwrap()))], [E231::Live(Wrap(black_box(NonZeroU32::new(1).unwrap()))), E231::Live(Wrap(black_box(NonZeroU32::new(0xFFFF_FFFF).unwrap())))], |e| match e { E231::U1 => 1002, E231::U0 => 1001, E231::D0(..) => 9001, E231::Live(w) => { let p: NonZeroU32 = w.0; p.get() } });
#[derive(Clone, Copy)] enum E232 { D0(AllDead), U1, U0, Live(Named<Option<bool>>) }
suite!(232, s232, E232, [E232::U1, E232::U0, E232::Live(Named { z: (), v: Some(true), p: PhantomData }), E232::Live(Named { z: (), v: Some(false), p: PhantomData }), E232::Live(Named { z: (), v: None, p: PhantomData })], [E232::Live(Named { z: (), v: black_box(Some(true)), p: PhantomData }), E232::Live(Named { z: (), v: black_box(Some(false)), p: PhantomData }), E232::Live(Named { z: (), v: black_box(None), p: PhantomData })], |e| match e { E232::D0(..) => 9001, E232::U1 => 1002, E232::U0 => 1001, E232::Live(w) => { let p: Option<bool> = w.v; match p { Some(b) => 40 + b as u32, None => 49 } } });
#[derive(Clone, Copy)] enum E233 { D0(NamedDead), U0, Live(ManuallyDrop<fn() -> u32>), D1([u32; 0], Void) }
suite!(233, s233, E233, [E233::U0, E233::Live(ManuallyDrop::new(seven)), E233::Live(ManuallyDrop::new(eight))], [E233::Live(ManuallyDrop::new(black_box(seven))), E233::Live(ManuallyDrop::new(black_box(eight)))], |e| match e { E233::D0(..) => 9001, E233::U0 => 1001, E233::Live(w) => { let p: fn() -> u32 = ManuallyDrop::into_inner(w); p() }, E233::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E234 { Live(ZPad<Ordering>), D0(AllDead) }
suite!(234, s234, E234, [E234::Live(ZPad([], Ordering::Less, ())), E234::Live(ZPad([], Ordering::Equal, ())), E234::Live(ZPad([], Ordering::Greater, ()))], [E234::Live(ZPad([], black_box(Ordering::Less), ())), E234::Live(ZPad([], black_box(Ordering::Equal), ())), E234::Live(ZPad([], black_box(Ordering::Greater), ()))], |e| match e { E234::Live(w) => { let p: Ordering = w.1; p as i32 as u32 }, E234::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E235 { D0((u32, Void)), Live(Tr<Hi>), U1, U0 }
suite!(235, s235, E235, [E235::Live(Tr(Hi::A)), E235::Live(Tr(Hi::B)), E235::U1, E235::U0], [E235::Live(Tr(black_box(Hi::A))), E235::Live(Tr(black_box(Hi::B)))], |e| match e { E235::D0(..) => 9001, E235::Live(w) => { let p: Hi = w.0; p as u32 }, E235::U1 => 1002, E235::U0 => 1001 });
#[derive(Clone, Copy)] enum E236 { Live(ZPad<Sg2>), D1(Result<(), Void>), D0(PkDead) }
suite!(236, s236, E236, [E236::Live(ZPad([], Sg2::A, ())), E236::Live(ZPad([], Sg2::B, ())), E236::D1(Ok(()))], [E236::Live(ZPad([], black_box(Sg2::A), ())), E236::Live(ZPad([], black_box(Sg2::B), ()))], |e| match e { E236::Live(w) => { let p: Sg2 = w.1; p as i32 as u32 }, E236::D1(..) => 2002, E236::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E237 { Live(Result<Never, fn() -> u32>), D0(Wrap<HalfDead>, ()), D1(Wrap<Void>), U0 }
suite!(237, s237, E237, [E237::Live(Err(seven)), E237::Live(Err(eight)), E237::D0(Wrap(HalfDead::B), ()), E237::U0], [E237::Live(Err(black_box(seven))), E237::Live(Err(black_box(eight)))], |e| match e { E237::Live(w) => { let p: fn() -> u32 = (match w { Err(x) => x, Ok(_) => loop {} }); p() }, E237::D0(..) => 2001, E237::D1(..) => 9002, E237::U0 => 1001 });
#[derive(Clone, Copy)] enum E238 { D2(MaybeUninit<Void>), Live(SL<NonZeroI8>), U2, U0, D0(Option<Void>), D1((), Infallible), U1 }
suite!(238, s238, E238, [E238::Live(SL::Live(NonZeroI8::new(-1).unwrap())), E238::Live(SL::Live(NonZeroI8::new(127).unwrap())), E238::U2, E238::U0, E238::D0(None), E238::U1], [E238::Live(SL::Live(black_box(NonZeroI8::new(-1).unwrap()))), E238::Live(SL::Live(black_box(NonZeroI8::new(127).unwrap())))], |e| match e { E238::D2(..) => 9003, E238::Live(w) => { let p: NonZeroI8 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); p.get() as i32 as u32 }, E238::U2 => 1003, E238::U0 => 1001, E238::D0(..) => 2001, E238::D1(..) => 9002, E238::U1 => 1002 });
#[derive(Clone, Copy)] enum E239 { U1, D0((Void,)), Live(CW<char>), U0 }
suite!(239, s239, E239, [E239::U1, E239::Live(CW('a')), E239::Live(CW('\u{10FFFF}')), E239::Live(CW('\0')), E239::U0], [E239::Live(CW(black_box('a'))), E239::Live(CW(black_box('\u{10FFFF}'))), E239::Live(CW(black_box('\0')))], |e| match e { E239::U1 => 1002, E239::D0(..) => 9001, E239::Live(w) => { let p: char = w.0; p as u32 }, E239::U0 => 1001 });

#[unsafe(no_mangle)]
pub fn rust_entry() {
    s220(); s221(); s222(); s223(); s224(); s225(); s226(); s227(); s228(); s229(); s230(); s231(); s232(); s233(); s234(); s235(); s236(); s237(); s238(); s239();
}
