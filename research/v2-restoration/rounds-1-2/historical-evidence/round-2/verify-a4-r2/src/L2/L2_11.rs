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
#[derive(Clone, Copy)] enum E220 { Live(CW<NonZeroU32>), D0(Result<Void, Never>), U1, U0 }
suite!(220, s220, E220, [E220::Live(CW(NonZeroU32::new(1).unwrap())), E220::Live(CW(NonZeroU32::new(0xFFFF_FFFF).unwrap())), E220::U1, E220::U0], |e| match e { E220::Live(w) => { let p: NonZeroU32 = w.0; p.get() }, E220::D0(..) => 9001, E220::U1 => 1002, E220::U0 => 1001 });
#[derive(Clone, Copy)] enum E221 { Live(Tr<S32>) }
suite!(221, s221, E221, [E221::Live(Tr(S32::A)), E221::Live(Tr(S32::B))], |e| match e { E221::Live(w) => { let p: S32 = w.0; p as i32 as u32 } });
#[derive(Clone, Copy)] enum E222 { D0([(Void,); 3]), D2(Infallible), D1(Wrap<AllDead>), U0, U1, U2, Live(Named<NonZeroI8>) }
suite!(222, s222, E222, [E222::U0, E222::U1, E222::U2, E222::Live(Named { z: (), v: NonZeroI8::new(-1).unwrap(), p: PhantomData }), E222::Live(Named { z: (), v: NonZeroI8::new(127).unwrap(), p: PhantomData })], |e| match e { E222::D0(..) => 9001, E222::D2(..) => 9003, E222::D1(..) => 9002, E222::U0 => 1001, E222::U1 => 1002, E222::U2 => 1003, E222::Live(w) => { let p: NonZeroI8 = w.v; p.get() as i32 as u32 } });
#[derive(Clone, Copy)] enum E223 { U1, U0, Live(SL<fn() -> u32>) }
suite!(223, s223, E223, [E223::U1, E223::U0, E223::Live(SL::Live(seven)), E223::Live(SL::Live(eight))], |e| match e { E223::U1 => 1002, E223::U0 => 1001, E223::Live(w) => { let p: fn() -> u32 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); p() } });
#[derive(Clone, Copy)] enum E224 { Live(Tr<Option<Option<Three>>>), D1(NamedDead), U2, U0, U3, U1, U4, D2(UDead), U5, D0([Void; 0]) }
suite!(224, s224, E224, [E224::Live(Tr(Some(Some(Three::C)))), E224::Live(Tr(Some(None))), E224::Live(Tr(None)), E224::U2, E224::U0, E224::U3, E224::U1, E224::U4, E224::U5, E224::D0([])], |e| match e { E224::Live(w) => { let p: Option<Option<Three>> = w.0; match p { Some(Some(t)) => t as u32 + 60, Some(None) => 46, None => 45 } }, E224::D1(..) => 9002, E224::U2 => 1003, E224::U0 => 1001, E224::U3 => 1004, E224::U1 => 1002, E224::U4 => 1005, E224::D2(..) => 9003, E224::U5 => 1006, E224::D0(..) => 2001 });
#[derive(Clone, Copy)] enum E225 { D0(()), U0, Live(ZPad<S64>), D2(TrDead), D1(ManuallyDrop<Void>) }
suite!(225, s225, E225, [E225::D0(()), E225::U0, E225::Live(ZPad([], S64::A, ())), E225::Live(ZPad([], S64::B, ()))], |e| match e { E225::D0(..) => 2001, E225::U0 => 1001, E225::Live(w) => { let p: S64 = w.1; (p as i64 as u64 % 1_000_003) as u32 }, E225::D2(..) => 9003, E225::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E226 { U0, U1, U2, Live(Pk<Hi2>) }
suite!(226, s226, E226, [E226::U0, E226::U1, E226::U2, E226::Live(Pk(Hi2::A)), E226::Live(Pk(Hi2::B))], |e| match e { E226::U0 => 1001, E226::U1 => 1002, E226::U2 => 1003, E226::Live(w) => { let p: Hi2 = { w.0 }; p as u32 } });
#[derive(Clone, Copy)] enum E227 { D0(UDead), Live(CW<Option<Hi2>>), U1, U0 }
suite!(227, s227, E227, [E227::Live(CW(Some(Hi2::A))), E227::Live(CW(Some(Hi2::B))), E227::Live(CW(None)), E227::U1, E227::U0], |e| match e { E227::D0(..) => 9001, E227::Live(w) => { let p: Option<Hi2> = w.0; match p { Some(h) => h as u32, None => 47 } }, E227::U1 => 1002, E227::U0 => 1001 });
#[derive(Clone, Copy)] enum E228 { U2, U5, Live(Wrap<NonZeroU8>), U4, U0, U3, U1, D0(ManuallyDrop<Void>) }
suite!(228, s228, E228, [E228::U2, E228::U5, E228::Live(Wrap(NonZeroU8::new(1).unwrap())), E228::Live(Wrap(NonZeroU8::new(255).unwrap())), E228::U4, E228::U0, E228::U3, E228::U1], |e| match e { E228::U2 => 1003, E228::U5 => 1006, E228::Live(w) => { let p: NonZeroU8 = w.0; p.get() as u32 }, E228::U4 => 1005, E228::U0 => 1001, E228::U3 => 1004, E228::U1 => 1002, E228::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E229 { U2, Live(Result<S16, Void>), U0, U1, U3, D1((u32, Void)), D0([u16; 0], Void) }
suite!(229, s229, E229, [E229::U2, E229::Live(Ok(S16::A)), E229::Live(Ok(S16::B)), E229::U0, E229::U1, E229::U3], |e| match e { E229::U2 => 1003, E229::Live(w) => { let p: S16 = (match w { Ok(x) => x, Err(_) => loop {} }); p as i32 as u32 }, E229::U0 => 1001, E229::U1 => 1002, E229::U3 => 1004, E229::D1(..) => 9002, E229::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E230 { Live(SL<char>), U4, U0, D0(Void, (), PhantomData<u8>), U5, D1(CDead), U3, U2, U1 }
suite!(230, s230, E230, [E230::Live(SL::Live('a')), E230::Live(SL::Live('\u{10FFFF}')), E230::Live(SL::Live('\0')), E230::U4, E230::U0, E230::U5, E230::U3, E230::U2, E230::U1], |e| match e { E230::Live(w) => { let p: char = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); p as u32 }, E230::U4 => 1005, E230::U0 => 1001, E230::D0(..) => 9001, E230::U5 => 1006, E230::D1(..) => 9002, E230::U3 => 1004, E230::U2 => 1003, E230::U1 => 1002 });
#[derive(Clone, Copy)] enum E231 { Live(SL<Hi3>), D0(()) }
suite!(231, s231, E231, [E231::Live(SL::Live(Hi3::A)), E231::Live(SL::Live(Hi3::B)), E231::Live(SL::Live(Hi3::C)), E231::D0(())], |e| match e { E231::Live(w) => { let p: Hi3 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); p as u32 }, E231::D0(..) => 2001 });
#[derive(Clone, Copy)] enum E232 { D0(CDead), Live(Wrap<Tr<((), Option<char>)>>), U0, D1(Void, (), PhantomData<u8>) }
suite!(232, s232, E232, [E232::Live(Wrap(Tr(((), Some('z'))))), E232::Live(Wrap(Tr(((), None)))), E232::U0], |e| match e { E232::D0(..) => 9001, E232::Live(w) => { let p: Option<char> = w.0.0.1; match p { Some(c) => c as u32, None => 48 } }, E232::U0 => 1001, E232::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E233 { D0(Void), U0, D1(HalfDead), Live(((), Ordering)), D2(Aligned) }
suite!(233, s233, E233, [E233::U0, E233::D1(HalfDead::B), E233::Live(((), Ordering::Less)), E233::Live(((), Ordering::Equal)), E233::Live(((), Ordering::Greater))], |e| match e { E233::D0(..) => 9001, E233::U0 => 1001, E233::D1(..) => 2002, E233::Live(w) => { let p: Ordering = w.1; p as i32 as u32 }, E233::D2(..) => 9003 });
#[derive(Clone, Copy)] enum E234 { Live(char), U2, D0([u32; 0], Void), D1(Result<(), Void>), D2(UDead), U3, U0, U5, U1, U4 }
suite!(234, s234, E234, [E234::Live('a'), E234::Live('\u{10FFFF}'), E234::Live('\0'), E234::U2, E234::D1(Ok(())), E234::U3, E234::U0, E234::U5, E234::U1, E234::U4], |e| match e { E234::Live(w) => { let p: char = w; p as u32 }, E234::U2 => 1003, E234::D0(..) => 9001, E234::D1(..) => 2002, E234::D2(..) => 9003, E234::U3 => 1004, E234::U0 => 1001, E234::U5 => 1006, E234::U1 => 1002, E234::U4 => 1005 });
#[derive(Clone, Copy)] enum E235 { U3, U1, Live(Result<Never, char>), U0, U2 }
suite!(235, s235, E235, [E235::U3, E235::U1, E235::Live(Err('a')), E235::Live(Err('\u{10FFFF}')), E235::Live(Err('\0')), E235::U0, E235::U2], |e| match e { E235::U3 => 1004, E235::U1 => 1002, E235::Live(w) => { let p: char = (match w { Err(x) => x, Ok(_) => loop {} }); p as u32 }, E235::U0 => 1001, E235::U2 => 1003 });
#[derive(Clone, Copy)] enum E236 { D0((Void, ())), U2, U3, Live(Wrap<WU>), U0, U1 }
suite!(236, s236, E236, [E236::U2, E236::U3, E236::Live(Wrap(WU::A)), E236::Live(Wrap(WU::B)), E236::U0, E236::U1], |e| match e { E236::D0(..) => 9001, E236::U2 => 1003, E236::U3 => 1004, E236::Live(w) => { let p: WU = w.0; p as usize as u32 }, E236::U0 => 1001, E236::U1 => 1002 });
#[derive(Clone, Copy)] enum E237 { D1(Aligned), D0(Wrap<AllDead>), D2(Option<Void>), Live(SL<Small>) }
suite!(237, s237, E237, [E237::D2(None), E237::Live(SL::Live(Small::X)), E237::Live(SL::Live(Small::Y))], |e| match e { E237::D1(..) => 9002, E237::D0(..) => 9001, E237::D2(..) => 2003, E237::Live(w) => { let p: Small = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); p as u32 } });
#[derive(Clone, Copy)] enum E238 { U1, D2(AllDead), D0(([Void; 0], Void)), D1(&'static Void, Void), U3, U0, Live(Named<Three>), U2 }
suite!(238, s238, E238, [E238::U1, E238::U3, E238::U0, E238::Live(Named { z: (), v: Three::A, p: PhantomData }), E238::Live(Named { z: (), v: Three::B, p: PhantomData }), E238::Live(Named { z: (), v: Three::C, p: PhantomData }), E238::U2], |e| match e { E238::U1 => 1002, E238::D2(..) => 9003, E238::D0(..) => 9001, E238::D1(..) => 9002, E238::U3 => 1004, E238::U0 => 1001, E238::Live(w) => { let p: Three = w.v; p as u32 + 20 }, E238::U2 => 1003 });
#[derive(Clone, Copy)] enum E239 { U0, Live(ZPad<Hi3>), D1(Never), D0(()) }
suite!(239, s239, E239, [E239::U0, E239::Live(ZPad([], Hi3::A, ())), E239::Live(ZPad([], Hi3::B, ())), E239::Live(ZPad([], Hi3::C, ())), E239::D0(())], |e| match e { E239::U0 => 1001, E239::Live(w) => { let p: Hi3 = w.1; p as u32 }, E239::D1(..) => 9002, E239::D0(..) => 2001 });

#[unsafe(no_mangle)]
pub fn rust_entry() {
    s220(); s221(); s222(); s223(); s224(); s225(); s226(); s227(); s228(); s229(); s230(); s231(); s232(); s233(); s234(); s235(); s236(); s237(); s238(); s239();
}
