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
#[derive(Clone, Copy)] enum E340 { U2, U3, Live(S64), D1(Never), U0, U1, D0(TrDead) }
suite!(340, s340, E340, [E340::U2, E340::U3, E340::Live(S64::A), E340::Live(S64::B), E340::U0, E340::U1], [E340::Live(black_box(S64::A)), E340::Live(black_box(S64::B))], |e| match e { E340::U2 => 1003, E340::U3 => 1004, E340::Live(w) => { let p: S64 = w; (p as i64 as u64 % 1_000_003) as u32 }, E340::D1(..) => 9002, E340::U0 => 1001, E340::U1 => 1002, E340::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E341 { D0([u32; 0], Void), U0, U2, Live(ManuallyDrop<Sg2>), U3, U1 }
suite!(341, s341, E341, [E341::U0, E341::U2, E341::Live(ManuallyDrop::new(Sg2::A)), E341::Live(ManuallyDrop::new(Sg2::B)), E341::U3, E341::U1], [E341::Live(ManuallyDrop::new(black_box(Sg2::A))), E341::Live(ManuallyDrop::new(black_box(Sg2::B)))], |e| match e { E341::D0(..) => 9001, E341::U0 => 1001, E341::U2 => 1003, E341::Live(w) => { let p: Sg2 = ManuallyDrop::into_inner(w); p as i32 as u32 }, E341::U3 => 1004, E341::U1 => 1002 });
#[derive(Clone, Copy)] enum E342 { Live(Result<Never, Ordering>), D0(PkDead) }
suite!(342, s342, E342, [E342::Live(Err(Ordering::Less)), E342::Live(Err(Ordering::Equal)), E342::Live(Err(Ordering::Greater))], [E342::Live(Err(black_box(Ordering::Less))), E342::Live(Err(black_box(Ordering::Equal))), E342::Live(Err(black_box(Ordering::Greater)))], |e| match e { E342::Live(w) => { let p: Ordering = (match w { Err(x) => x, Ok(_) => loop {} }); p as i32 as u32 }, E342::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E343 { Live(Result<Sg3, Void>), D1((Void,)), D0(([Void; 0], Void)) }
suite!(343, s343, E343, [E343::Live(Ok(Sg3::A)), E343::Live(Ok(Sg3::B))], [E343::Live(Ok(black_box(Sg3::A))), E343::Live(Ok(black_box(Sg3::B)))], |e| match e { E343::Live(w) => { let p: Sg3 = (match w { Ok(x) => x, Err(_) => loop {} }); p as i32 as u32 }, E343::D1(..) => 9002, E343::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E344 { D2(AllDead), D1(HalfDead2), Live(ZPad<Sg2>), U0, D0((Never, Never)) }
suite!(344, s344, E344, [E344::D1(HalfDead2::A), E344::Live(ZPad([], Sg2::A, ())), E344::Live(ZPad([], Sg2::B, ())), E344::U0], [E344::Live(ZPad([], black_box(Sg2::A), ())), E344::Live(ZPad([], black_box(Sg2::B), ()))], |e| match e { E344::D2(..) => 9003, E344::D1(..) => 2002, E344::Live(w) => { let p: Sg2 = w.1; p as i32 as u32 }, E344::U0 => 1001, E344::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E345 { Live(NonZeroU8), D1(PkDead), D0(Wrap<HalfDead>, ()) }
suite!(345, s345, E345, [E345::Live(NonZeroU8::new(1).unwrap()), E345::Live(NonZeroU8::new(255).unwrap()), E345::D0(Wrap(HalfDead::B), ())], [E345::Live(black_box(NonZeroU8::new(1).unwrap())), E345::Live(black_box(NonZeroU8::new(255).unwrap()))], |e| match e { E345::Live(w) => { let p: NonZeroU8 = w; p.get() as u32 }, E345::D1(..) => 9002, E345::D0(..) => 2001 });
#[derive(Clone, Copy)] enum E346 { U0, Live([S32; 1]), D0(PhantomData<Void>), U2, U1 }
suite!(346, s346, E346, [E346::U0, E346::Live([S32::A]), E346::Live([S32::B]), E346::D0(PhantomData), E346::U2, E346::U1], [E346::Live([black_box(S32::A)]), E346::Live([black_box(S32::B)])], |e| match e { E346::U0 => 1001, E346::Live(w) => { let p: S32 = w[0]; p as i32 as u32 }, E346::D0(..) => 2001, E346::U2 => 1003, E346::U1 => 1002 });
#[derive(Clone, Copy)] enum E347 { D0([u32; 0], Void), Live(Result<Never, Sg2>) }
suite!(347, s347, E347, [E347::Live(Err(Sg2::A)), E347::Live(Err(Sg2::B))], [E347::Live(Err(black_box(Sg2::A))), E347::Live(Err(black_box(Sg2::B)))], |e| match e { E347::D0(..) => 9001, E347::Live(w) => { let p: Sg2 = (match w { Err(x) => x, Ok(_) => loop {} }); p as i32 as u32 } });
#[derive(Clone, Copy)] enum E348 { U1, U0, Live((fn() -> u32,)) }
suite!(348, s348, E348, [E348::U1, E348::U0, E348::Live((seven,)), E348::Live((eight,))], [E348::Live((black_box(seven),)), E348::Live((black_box(eight),))], |e| match e { E348::U1 => 1002, E348::U0 => 1001, E348::Live(w) => { let p: fn() -> u32 = w.0; p() } });
#[derive(Clone, Copy)] enum E349 { U0, Live(Result<Never, S16>), U2, D0(Void), U1 }
suite!(349, s349, E349, [E349::U0, E349::Live(Err(S16::A)), E349::Live(Err(S16::B)), E349::U2, E349::U1], [E349::Live(Err(black_box(S16::A))), E349::Live(Err(black_box(S16::B)))], |e| match e { E349::U0 => 1001, E349::Live(w) => { let p: S16 = (match w { Err(x) => x, Ok(_) => loop {} }); p as i32 as u32 }, E349::U2 => 1003, E349::D0(..) => 9001, E349::U1 => 1002 });
#[derive(Clone, Copy)] enum E350 { U2, Live(S16), U0, U3, U1 }
suite!(350, s350, E350, [E350::U2, E350::Live(S16::A), E350::Live(S16::B), E350::U0, E350::U3, E350::U1], [E350::Live(black_box(S16::A)), E350::Live(black_box(S16::B))], |e| match e { E350::U2 => 1003, E350::Live(w) => { let p: S16 = w; p as i32 as u32 }, E350::U0 => 1001, E350::U3 => 1004, E350::U1 => 1002 });
#[derive(Clone, Copy)] enum E351 { D0(Void, (), PhantomData<u8>), Live(Wrap<Tr<((), NonZeroU16)>>) }
suite!(351, s351, E351, [E351::Live(Wrap(Tr(((), NonZeroU16::new(1).unwrap())))), E351::Live(Wrap(Tr(((), NonZeroU16::new(65535).unwrap()))))], [E351::Live(Wrap(Tr(((), black_box(NonZeroU16::new(1).unwrap()))))), E351::Live(Wrap(Tr(((), black_box(NonZeroU16::new(65535).unwrap())))))], |e| match e { E351::D0(..) => 9001, E351::Live(w) => { let p: NonZeroU16 = w.0.0.1; p.get() as u32 } });
#[derive(Clone, Copy)] enum E352 { Live(SL<Wrap<SL3<Option<Hi2>>>>), D1([u32; 0]), D0([Void; 0]), U0 }
suite!(352, s352, E352, [E352::Live(SL::Live(Wrap(SL3::Live(Some(Hi2::A))))), E352::Live(SL::Live(Wrap(SL3::Live(Some(Hi2::B))))), E352::Live(SL::Live(Wrap(SL3::Live(None)))), E352::D1([]), E352::D0([]), E352::U0], [E352::Live(SL::Live(Wrap(SL3::Live(black_box(Some(Hi2::A)))))), E352::Live(SL::Live(Wrap(SL3::Live(black_box(Some(Hi2::B)))))), E352::Live(SL::Live(Wrap(SL3::Live(black_box(None)))))], |e| match e { E352::Live(w) => { let p: Option<Hi2> = (match w { SL::Live(Wrap(SL3::Live(x))) => x, _ => loop {} }); match p { Some(h) => h as u32, None => 47 } }, E352::D1(..) => 2002, E352::D0(..) => 2001, E352::U0 => 1001 });
#[derive(Clone, Copy)] enum E353 { Live(Pk<Hi>), D0(Void, (), PhantomData<u8>), U2, U0, U5, U4, U1, U3 }
suite!(353, s353, E353, [E353::Live(Pk(Hi::A)), E353::Live(Pk(Hi::B)), E353::U2, E353::U0, E353::U5, E353::U4, E353::U1, E353::U3], [E353::Live(Pk(black_box(Hi::A))), E353::Live(Pk(black_box(Hi::B)))], |e| match e { E353::Live(w) => { let p: Hi = { w.0 }; p as u32 }, E353::D0(..) => 9001, E353::U2 => 1003, E353::U0 => 1001, E353::U5 => 1006, E353::U4 => 1005, E353::U1 => 1002, E353::U3 => 1004 });
#[derive(Clone, Copy)] enum E354 { Live(ZPad<Hi>), D1((Void, ())), U0, D0(Wrap<Void>) }
suite!(354, s354, E354, [E354::Live(ZPad([], Hi::A, ())), E354::Live(ZPad([], Hi::B, ())), E354::U0], [E354::Live(ZPad([], black_box(Hi::A), ())), E354::Live(ZPad([], black_box(Hi::B), ()))], |e| match e { E354::Live(w) => { let p: Hi = w.1; p as u32 }, E354::D1(..) => 9002, E354::U0 => 1001, E354::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E355 { Live(Option<char>), U1, U0, D1((u32, Void)), D0(([Void; 0], Void)), D2(Infallible), U2 }
suite!(355, s355, E355, [E355::Live(Some('z')), E355::Live(None), E355::U1, E355::U0, E355::U2], [E355::Live(black_box(Some('z'))), E355::Live(black_box(None))], |e| match e { E355::Live(w) => { let p: Option<char> = w; match p { Some(c) => c as u32, None => 48 } }, E355::U1 => 1002, E355::U0 => 1001, E355::D1(..) => 9002, E355::D0(..) => 9001, E355::D2(..) => 9003, E355::U2 => 1003 });
#[derive(Clone, Copy)] enum E356 { U0, Live(ZPad<Two>) }
suite!(356, s356, E356, [E356::U0, E356::Live(ZPad([], Two::A, ())), E356::Live(ZPad([], Two::B, ()))], [E356::Live(ZPad([], black_box(Two::A), ())), E356::Live(ZPad([], black_box(Two::B), ()))], |e| match e { E356::U0 => 1001, E356::Live(w) => { let p: Two = w.1; p as u32 + 30 } });
#[derive(Clone, Copy)] enum E357 { Live(NonZeroU8), U0, U1, D0(Result<(), Void>) }
suite!(357, s357, E357, [E357::Live(NonZeroU8::new(1).unwrap()), E357::Live(NonZeroU8::new(255).unwrap()), E357::U0, E357::U1, E357::D0(Ok(()))], [E357::Live(black_box(NonZeroU8::new(1).unwrap())), E357::Live(black_box(NonZeroU8::new(255).unwrap()))], |e| match e { E357::Live(w) => { let p: NonZeroU8 = w; p.get() as u32 }, E357::U0 => 1001, E357::U1 => 1002, E357::D0(..) => 2001 });
#[derive(Clone, Copy)] enum E358 { Live((Option<Hi2>,)) }
suite!(358, s358, E358, [E358::Live((Some(Hi2::A),)), E358::Live((Some(Hi2::B),)), E358::Live((None,))], [E358::Live((black_box(Some(Hi2::A)),)), E358::Live((black_box(Some(Hi2::B)),)), E358::Live((black_box(None),))], |e| match e { E358::Live(w) => { let p: Option<Hi2> = w.0; match p { Some(h) => h as u32, None => 47 } } });
#[derive(Clone, Copy)] enum E359 { Live(Pk<Option<bool>>), U0, D1(DeepDead), U1, D0(MaybeUninit<Void>) }
suite!(359, s359, E359, [E359::Live(Pk(Some(true))), E359::Live(Pk(Some(false))), E359::Live(Pk(None)), E359::U0, E359::U1], [E359::Live(Pk(black_box(Some(true)))), E359::Live(Pk(black_box(Some(false)))), E359::Live(Pk(black_box(None)))], |e| match e { E359::Live(w) => { let p: Option<bool> = { w.0 }; match p { Some(b) => 40 + b as u32, None => 49 } }, E359::U0 => 1001, E359::D1(..) => 9002, E359::U1 => 1002, E359::D0(..) => 9001 });

#[unsafe(no_mangle)]
pub fn rust_entry() {
    s340(); s341(); s342(); s343(); s344(); s345(); s346(); s347(); s348(); s349(); s350(); s351(); s352(); s353(); s354(); s355(); s356(); s357(); s358(); s359();
}
