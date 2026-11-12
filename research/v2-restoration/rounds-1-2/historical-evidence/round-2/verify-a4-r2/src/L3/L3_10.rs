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
#[derive(Clone, Copy)] enum E200 { Live(Wrap<Tr<((), WU)>>), U0, U2, D0([u32; 0], Void), D1(Infallible), U1 }
suite!(200, s200, E200, [E200::Live(Wrap(Tr(((), WU::A)))), E200::Live(Wrap(Tr(((), WU::B)))), E200::U0, E200::U2, E200::U1], [E200::Live(Wrap(Tr(((), black_box(WU::A))))), E200::Live(Wrap(Tr(((), black_box(WU::B)))))], |e| match e { E200::Live(w) => { let p: WU = w.0.0.1; p as usize as u32 }, E200::U0 => 1001, E200::U2 => 1003, E200::D0(..) => 9001, E200::D1(..) => 9002, E200::U1 => 1002 });
#[derive(Clone, Copy)] enum E201 { D2(TrDead), U2, U1, D0(Infallible), D1(UDead), U0, Live(CW<bool>) }
suite!(201, s201, E201, [E201::U2, E201::U1, E201::U0, E201::Live(CW(true)), E201::Live(CW(false))], [E201::Live(CW(black_box(true))), E201::Live(CW(black_box(false)))], |e| match e { E201::D2(..) => 9003, E201::U2 => 1003, E201::U1 => 1002, E201::D0(..) => 9001, E201::D1(..) => 9002, E201::U0 => 1001, E201::Live(w) => { let p: bool = w.0; 10 + p as u32 } });
#[derive(Clone, Copy)] enum E202 { U0, U1, D0([Void; 2]), Live(Wrap<Tr<((), W32)>>) }
suite!(202, s202, E202, [E202::U0, E202::U1, E202::Live(Wrap(Tr(((), W32::A)))), E202::Live(Wrap(Tr(((), W32::B))))], [E202::Live(Wrap(Tr(((), black_box(W32::A))))), E202::Live(Wrap(Tr(((), black_box(W32::B)))))], |e| match e { E202::U0 => 1001, E202::U1 => 1002, E202::D0(..) => 9001, E202::Live(w) => { let p: W32 = w.0.0.1; p as u32 } });
#[derive(Clone, Copy)] enum E203 { D0(Wrap<HalfDead>, ()), U0, U1, Live(SL<W16>) }
suite!(203, s203, E203, [E203::D0(Wrap(HalfDead::B), ()), E203::U0, E203::U1, E203::Live(SL::Live(W16::A)), E203::Live(SL::Live(W16::B))], [E203::Live(SL::Live(black_box(W16::A))), E203::Live(SL::Live(black_box(W16::B)))], |e| match e { E203::D0(..) => 2001, E203::U0 => 1001, E203::U1 => 1002, E203::Live(w) => { let p: W16 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); p as u32 } });
#[derive(Clone, Copy)] enum E204 { U0, D0([(Void,); 0]), Live(SL<Wrap<SL3<fn() -> u32>>>) }
suite!(204, s204, E204, [E204::U0, E204::D0([]), E204::Live(SL::Live(Wrap(SL3::Live(seven)))), E204::Live(SL::Live(Wrap(SL3::Live(eight))))], [E204::Live(SL::Live(Wrap(SL3::Live(black_box(seven))))), E204::Live(SL::Live(Wrap(SL3::Live(black_box(eight)))))], |e| match e { E204::U0 => 1001, E204::D0(..) => 2001, E204::Live(w) => { let p: fn() -> u32 = (match w { SL::Live(Wrap(SL3::Live(x))) => x, _ => loop {} }); p() } });
#[derive(Clone, Copy)] enum E205 { Live(Tr<NonZeroU8>) }
suite!(205, s205, E205, [E205::Live(Tr(NonZeroU8::new(1).unwrap())), E205::Live(Tr(NonZeroU8::new(255).unwrap()))], [E205::Live(Tr(black_box(NonZeroU8::new(1).unwrap()))), E205::Live(Tr(black_box(NonZeroU8::new(255).unwrap())))], |e| match e { E205::Live(w) => { let p: NonZeroU8 = w.0; p.get() as u32 } });
#[derive(Clone, Copy)] enum E206 { U2, U5, U4, U3, Live(Three), U0, U1, D1(Void, [u64; 0]), D0(Void) }
suite!(206, s206, E206, [E206::U2, E206::U5, E206::U4, E206::U3, E206::Live(Three::A), E206::Live(Three::B), E206::Live(Three::C), E206::U0, E206::U1], [E206::Live(black_box(Three::A)), E206::Live(black_box(Three::B)), E206::Live(black_box(Three::C))], |e| match e { E206::U2 => 1003, E206::U5 => 1006, E206::U4 => 1005, E206::U3 => 1004, E206::Live(w) => { let p: Three = w; p as u32 + 20 }, E206::U0 => 1001, E206::U1 => 1002, E206::D1(..) => 9002, E206::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E207 { U3, D1(Wrap<HalfDead>, ()), U1, U5, U2, D0(Wrap<Wrap<(Void, ())>>), Live((bool,)), U0, D2(UDead), U4 }
suite!(207, s207, E207, [E207::U3, E207::D1(Wrap(HalfDead::B), ()), E207::U1, E207::U5, E207::U2, E207::Live((true,)), E207::Live((false,)), E207::U0, E207::U4], [E207::Live((black_box(true),)), E207::Live((black_box(false),))], |e| match e { E207::U3 => 1004, E207::D1(..) => 2002, E207::U1 => 1002, E207::U5 => 1006, E207::U2 => 1003, E207::D0(..) => 9001, E207::Live(w) => { let p: bool = w.0; 10 + p as u32 }, E207::U0 => 1001, E207::D2(..) => 9003, E207::U4 => 1005 });
#[derive(Clone, Copy)] enum E208 { U2, U3, D0(Wrap<AllDead>), U0, U1, Live(SL3<NonZeroU64>) }
suite!(208, s208, E208, [E208::U2, E208::U3, E208::U0, E208::U1, E208::Live(SL3::Live(NonZeroU64::new(1).unwrap())), E208::Live(SL3::Live(NonZeroU64::new(0xFFFF_FFFF_FFFF_FFFF).unwrap())), E208::Live(SL3::Live(NonZeroU64::new(0x1_0000_0000).unwrap()))], [E208::Live(SL3::Live(black_box(NonZeroU64::new(1).unwrap()))), E208::Live(SL3::Live(black_box(NonZeroU64::new(0xFFFF_FFFF_FFFF_FFFF).unwrap()))), E208::Live(SL3::Live(black_box(NonZeroU64::new(0x1_0000_0000).unwrap())))], |e| match e { E208::U2 => 1003, E208::U3 => 1004, E208::D0(..) => 9001, E208::U0 => 1001, E208::U1 => 1002, E208::Live(w) => { let p: NonZeroU64 = (match w { SL3::Live(x) => x, _ => loop {} }); (p.get() % 1_000_003) as u32 } });
#[derive(Clone, Copy)] enum E209 { U0, U2, U1, Live(Wrap<Sg>), D0(PkDead) }
suite!(209, s209, E209, [E209::U0, E209::U2, E209::U1, E209::Live(Wrap(Sg::A)), E209::Live(Wrap(Sg::B))], [E209::Live(Wrap(black_box(Sg::A))), E209::Live(Wrap(black_box(Sg::B)))], |e| match e { E209::U0 => 1001, E209::U2 => 1003, E209::U1 => 1002, E209::Live(w) => { let p: Sg = w.0; p as i32 as u32 }, E209::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E210 { U0, U2, Live(NonZeroI8), U1 }
suite!(210, s210, E210, [E210::U0, E210::U2, E210::Live(NonZeroI8::new(-1).unwrap()), E210::Live(NonZeroI8::new(127).unwrap()), E210::U1], [E210::Live(black_box(NonZeroI8::new(-1).unwrap())), E210::Live(black_box(NonZeroI8::new(127).unwrap()))], |e| match e { E210::U0 => 1001, E210::U2 => 1003, E210::Live(w) => { let p: NonZeroI8 = w; p.get() as i32 as u32 }, E210::U1 => 1002 });
#[derive(Clone, Copy)] enum E211 { Live(Wrap<Tr<((), Sg3)>>), U3, U1, U2, U0, D0([Void; 2]), U4, U5 }
suite!(211, s211, E211, [E211::Live(Wrap(Tr(((), Sg3::A)))), E211::Live(Wrap(Tr(((), Sg3::B)))), E211::U3, E211::U1, E211::U2, E211::U0, E211::U4, E211::U5], [E211::Live(Wrap(Tr(((), black_box(Sg3::A))))), E211::Live(Wrap(Tr(((), black_box(Sg3::B)))))], |e| match e { E211::Live(w) => { let p: Sg3 = w.0.0.1; p as i32 as u32 }, E211::U3 => 1004, E211::U1 => 1002, E211::U2 => 1003, E211::U0 => 1001, E211::D0(..) => 9001, E211::U4 => 1005, E211::U5 => 1006 });
#[derive(Clone, Copy)] enum E212 { U4, Live(SL3<NonZeroI16>), U1, U2, D1(ManuallyDrop<Void>), U5, D0(()), U3, U0 }
suite!(212, s212, E212, [E212::U4, E212::Live(SL3::Live(NonZeroI16::new(-1).unwrap())), E212::Live(SL3::Live(NonZeroI16::new(300).unwrap())), E212::U1, E212::U2, E212::U5, E212::D0(()), E212::U3, E212::U0], [E212::Live(SL3::Live(black_box(NonZeroI16::new(-1).unwrap()))), E212::Live(SL3::Live(black_box(NonZeroI16::new(300).unwrap())))], |e| match e { E212::U4 => 1005, E212::Live(w) => { let p: NonZeroI16 = (match w { SL3::Live(x) => x, _ => loop {} }); p.get() as i32 as u32 }, E212::U1 => 1002, E212::U2 => 1003, E212::D1(..) => 9002, E212::U5 => 1006, E212::D0(..) => 2001, E212::U3 => 1004, E212::U0 => 1001 });
#[derive(Clone, Copy)] enum E213 { D1([Void; 2]), U0, D0(&'static Void, Void), Live(Named<NonZeroI16>), U1 }
suite!(213, s213, E213, [E213::U0, E213::Live(Named { z: (), v: NonZeroI16::new(-1).unwrap(), p: PhantomData }), E213::Live(Named { z: (), v: NonZeroI16::new(300).unwrap(), p: PhantomData }), E213::U1], [E213::Live(Named { z: (), v: black_box(NonZeroI16::new(-1).unwrap()), p: PhantomData }), E213::Live(Named { z: (), v: black_box(NonZeroI16::new(300).unwrap()), p: PhantomData })], |e| match e { E213::D1(..) => 9002, E213::U0 => 1001, E213::D0(..) => 9001, E213::Live(w) => { let p: NonZeroI16 = w.v; p.get() as i32 as u32 }, E213::U1 => 1002 });
#[derive(Clone, Copy)] enum E214 { U1, U0, U2, Live(Named<&'static u32>) }
suite!(214, s214, E214, [E214::U1, E214::U0, E214::U2, E214::Live(Named { z: (), v: &5, p: PhantomData }), E214::Live(Named { z: (), v: &SEVEN, p: PhantomData })], [E214::Live(Named { z: (), v: black_box(&5), p: PhantomData }), E214::Live(Named { z: (), v: black_box(&SEVEN), p: PhantomData })], |e| match e { E214::U1 => 1002, E214::U0 => 1001, E214::U2 => 1003, E214::Live(w) => { let p: &'static u32 = w.v; *p } });
#[derive(Clone, Copy)] enum E215 { D0(([Void; 0], Void)), D1(PkDead), Live(W64), D2(ManuallyDrop<Void>) }
suite!(215, s215, E215, [E215::Live(W64::A), E215::Live(W64::B)], [E215::Live(black_box(W64::A)), E215::Live(black_box(W64::B))], |e| match e { E215::D0(..) => 9001, E215::D1(..) => 9002, E215::Live(w) => { let p: W64 = w; (p as u64 % 1_000_003) as u32 }, E215::D2(..) => 9003 });
#[derive(Clone, Copy)] enum E216 { U3, Live(Result<fn() -> u32, Void>), U2, D0([(Void,); 3]), U0, U1 }
suite!(216, s216, E216, [E216::U3, E216::Live(Ok(seven)), E216::Live(Ok(eight)), E216::U2, E216::U0, E216::U1], [E216::Live(Ok(black_box(seven))), E216::Live(Ok(black_box(eight)))], |e| match e { E216::U3 => 1004, E216::Live(w) => { let p: fn() -> u32 = (match w { Ok(x) => x, Err(_) => loop {} }); p() }, E216::U2 => 1003, E216::D0(..) => 9001, E216::U0 => 1001, E216::U1 => 1002 });
#[derive(Clone, Copy)] enum E217 { D2(Void, [u64; 0]), D1([(Void,); 3]), Live(Option<Option<Three>>), D0(()) }
suite!(217, s217, E217, [E217::Live(Some(Some(Three::C))), E217::Live(Some(None)), E217::Live(None), E217::D0(())], [E217::Live(black_box(Some(Some(Three::C)))), E217::Live(black_box(Some(None))), E217::Live(black_box(None))], |e| match e { E217::D2(..) => 9003, E217::D1(..) => 9002, E217::Live(w) => { let p: Option<Option<Three>> = w; match p { Some(Some(t)) => t as u32 + 60, Some(None) => 46, None => 45 } }, E217::D0(..) => 2001 });
#[derive(Clone, Copy)] enum E218 { D1(PhantomData<Void>), D2(PhantomData<Void>), D0(Void), Live(Result<Never, Two>), U0 }
suite!(218, s218, E218, [E218::D1(PhantomData), E218::D2(PhantomData), E218::Live(Err(Two::A)), E218::Live(Err(Two::B)), E218::U0], [E218::Live(Err(black_box(Two::A))), E218::Live(Err(black_box(Two::B)))], |e| match e { E218::D1(..) => 2002, E218::D2(..) => 2003, E218::D0(..) => 9001, E218::Live(w) => { let p: Two = (match w { Err(x) => x, Ok(_) => loop {} }); p as u32 + 30 }, E218::U0 => 1001 });
#[derive(Clone, Copy)] enum E219 { D1(HalfDead2), U0, U1, D2([(Void,); 3]), Live(W16), D0((HalfDead, Void)) }
suite!(219, s219, E219, [E219::D1(HalfDead2::A), E219::U0, E219::U1, E219::Live(W16::A), E219::Live(W16::B)], [E219::Live(black_box(W16::A)), E219::Live(black_box(W16::B))], |e| match e { E219::D1(..) => 2002, E219::U0 => 1001, E219::U1 => 1002, E219::D2(..) => 9003, E219::Live(w) => { let p: W16 = w; p as u32 }, E219::D0(..) => 9001 });

#[unsafe(no_mangle)]
pub fn rust_entry() {
    s200(); s201(); s202(); s203(); s204(); s205(); s206(); s207(); s208(); s209(); s210(); s211(); s212(); s213(); s214(); s215(); s216(); s217(); s218(); s219();
}
