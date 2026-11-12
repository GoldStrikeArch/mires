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
#[derive(Clone, Copy)] enum E180 { D0(AllDead), Live(CW<Sg3>), U0 }
suite!(180, s180, E180, [E180::Live(CW(Sg3::A)), E180::Live(CW(Sg3::B)), E180::U0], [E180::Live(CW(black_box(Sg3::A))), E180::Live(CW(black_box(Sg3::B)))], |e| match e { E180::D0(..) => 9001, E180::Live(w) => { let p: Sg3 = w.0; p as i32 as u32 }, E180::U0 => 1001 });
#[derive(Clone, Copy)] enum E181 { Live((&'static u32, PhantomData<Void>, [Void; 0])), U0, D0(MaybeUninit<Void>) }
suite!(181, s181, E181, [E181::Live((&5, PhantomData, [])), E181::Live((&SEVEN, PhantomData, [])), E181::U0], [E181::Live((black_box(&5), PhantomData, [])), E181::Live((black_box(&SEVEN), PhantomData, []))], |e| match e { E181::Live(w) => { let p: &'static u32 = w.0; *p }, E181::U0 => 1001, E181::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E182 { D2(Wrap<Void>), Live(SL<W32>), U0, D1(HalfDead), D0((u32, Void)) }
suite!(182, s182, E182, [E182::Live(SL::Live(W32::A)), E182::Live(SL::Live(W32::B)), E182::U0, E182::D1(HalfDead::B)], [E182::Live(SL::Live(black_box(W32::A))), E182::Live(SL::Live(black_box(W32::B)))], |e| match e { E182::D2(..) => 9003, E182::Live(w) => { let p: W32 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); p as u32 }, E182::U0 => 1001, E182::D1(..) => 2002, E182::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E183 { Live(W16), U0, D0([Void; 0]) }
suite!(183, s183, E183, [E183::Live(W16::A), E183::Live(W16::B), E183::U0, E183::D0([])], [E183::Live(black_box(W16::A)), E183::Live(black_box(W16::B))], |e| match e { E183::Live(w) => { let p: W16 = w; p as u32 }, E183::U0 => 1001, E183::D0(..) => 2001 });
#[derive(Clone, Copy)] enum E184 { D1([[Void; 1]; 1]), D0([u32; 0], Void), U1, Live([NonZeroU64; 1]), U0, U2, U3, U5, U4 }
suite!(184, s184, E184, [E184::U1, E184::Live([NonZeroU64::new(1).unwrap()]), E184::Live([NonZeroU64::new(0xFFFF_FFFF_FFFF_FFFF).unwrap()]), E184::Live([NonZeroU64::new(0x1_0000_0000).unwrap()]), E184::U0, E184::U2, E184::U3, E184::U5, E184::U4], [E184::Live([black_box(NonZeroU64::new(1).unwrap())]), E184::Live([black_box(NonZeroU64::new(0xFFFF_FFFF_FFFF_FFFF).unwrap())]), E184::Live([black_box(NonZeroU64::new(0x1_0000_0000).unwrap())])], |e| match e { E184::D1(..) => 9002, E184::D0(..) => 9001, E184::U1 => 1002, E184::Live(w) => { let p: NonZeroU64 = w[0]; (p.get() % 1_000_003) as u32 }, E184::U0 => 1001, E184::U2 => 1003, E184::U3 => 1004, E184::U5 => 1006, E184::U4 => 1005 });
#[derive(Clone, Copy)] enum E185 { D0(ManuallyDrop<Void>), Live((S32,)) }
suite!(185, s185, E185, [E185::Live((S32::A,)), E185::Live((S32::B,))], [E185::Live((black_box(S32::A),)), E185::Live((black_box(S32::B),))], |e| match e { E185::D0(..) => 9001, E185::Live(w) => { let p: S32 = w.0; p as i32 as u32 } });
#[derive(Clone, Copy)] enum E186 { D1(Result<(), Void>), Live(ManuallyDrop<W16>), U0, D0(([Void; 0], Void)) }
suite!(186, s186, E186, [E186::D1(Ok(())), E186::Live(ManuallyDrop::new(W16::A)), E186::Live(ManuallyDrop::new(W16::B)), E186::U0], [E186::Live(ManuallyDrop::new(black_box(W16::A))), E186::Live(ManuallyDrop::new(black_box(W16::B)))], |e| match e { E186::D1(..) => 2002, E186::Live(w) => { let p: W16 = ManuallyDrop::into_inner(w); p as u32 }, E186::U0 => 1001, E186::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E187 { Live(Pk<fn() -> u32>), D0(HalfDead), D2(Wrap<Wrap<(Void, ())>>), D1(Wrap<AllDead>) }
suite!(187, s187, E187, [E187::Live(Pk(seven)), E187::Live(Pk(eight)), E187::D0(HalfDead::B)], [E187::Live(Pk(black_box(seven))), E187::Live(Pk(black_box(eight)))], |e| match e { E187::Live(w) => { let p: fn() -> u32 = { w.0 }; p() }, E187::D0(..) => 2001, E187::D2(..) => 9003, E187::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E188 { Live(((), fn() -> u32)), D0((HalfDead, Void)), D1(&'static Void, Void) }
suite!(188, s188, E188, [E188::Live(((), seven)), E188::Live(((), eight))], [E188::Live(((), black_box(seven))), E188::Live(((), black_box(eight)))], |e| match e { E188::Live(w) => { let p: fn() -> u32 = w.1; p() }, E188::D0(..) => 9001, E188::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E189 { D0(Option<Void>), Live(Wrap<Tr<((), Hi3)>>), U0 }
suite!(189, s189, E189, [E189::D0(None), E189::Live(Wrap(Tr(((), Hi3::A)))), E189::Live(Wrap(Tr(((), Hi3::B)))), E189::Live(Wrap(Tr(((), Hi3::C)))), E189::U0], [E189::Live(Wrap(Tr(((), black_box(Hi3::A))))), E189::Live(Wrap(Tr(((), black_box(Hi3::B))))), E189::Live(Wrap(Tr(((), black_box(Hi3::C)))))], |e| match e { E189::D0(..) => 2001, E189::Live(w) => { let p: Hi3 = w.0.0.1; p as u32 }, E189::U0 => 1001 });
#[derive(Clone, Copy)] enum E190 { D0(Wrap<Wrap<(Void, ())>>), Live([fn() -> u32; 1]) }
suite!(190, s190, E190, [E190::Live([seven]), E190::Live([eight])], [E190::Live([black_box(seven)]), E190::Live([black_box(eight)])], |e| match e { E190::D0(..) => 9001, E190::Live(w) => { let p: fn() -> u32 = w[0]; p() } });
#[derive(Clone, Copy)] enum E191 { D0(Wrap<AllDead>), Live(Result<Option<Hi2>, Void>), U0, D1(HalfDead2) }
suite!(191, s191, E191, [E191::Live(Ok(Some(Hi2::A))), E191::Live(Ok(Some(Hi2::B))), E191::Live(Ok(None)), E191::U0, E191::D1(HalfDead2::A)], [E191::Live(Ok(black_box(Some(Hi2::A)))), E191::Live(Ok(black_box(Some(Hi2::B)))), E191::Live(Ok(black_box(None)))], |e| match e { E191::D0(..) => 9001, E191::Live(w) => { let p: Option<Hi2> = (match w { Ok(x) => x, Err(_) => loop {} }); match p { Some(h) => h as u32, None => 47 } }, E191::U0 => 1001, E191::D1(..) => 2002 });
#[derive(Clone, Copy)] enum E192 { D2(Aligned), D0(Void, [u64; 0]), D1(PhantomData<Void>), Live((S32, PhantomData<Void>, [Void; 0])) }
suite!(192, s192, E192, [E192::D1(PhantomData), E192::Live((S32::A, PhantomData, [])), E192::Live((S32::B, PhantomData, []))], [E192::Live((black_box(S32::A), PhantomData, [])), E192::Live((black_box(S32::B), PhantomData, []))], |e| match e { E192::D2(..) => 9003, E192::D0(..) => 9001, E192::D1(..) => 2002, E192::Live(w) => { let p: S32 = w.0; p as i32 as u32 } });
#[derive(Clone, Copy)] enum E193 { Live(CW<WU>) }
suite!(193, s193, E193, [E193::Live(CW(WU::A)), E193::Live(CW(WU::B))], [E193::Live(CW(black_box(WU::A))), E193::Live(CW(black_box(WU::B)))], |e| match e { E193::Live(w) => { let p: WU = w.0; p as usize as u32 } });
#[derive(Clone, Copy)] enum E194 { Live(ManuallyDrop<W32>), D0([(Void,); 3]), D1(CDead) }
suite!(194, s194, E194, [E194::Live(ManuallyDrop::new(W32::A)), E194::Live(ManuallyDrop::new(W32::B))], [E194::Live(ManuallyDrop::new(black_box(W32::A))), E194::Live(ManuallyDrop::new(black_box(W32::B)))], |e| match e { E194::Live(w) => { let p: W32 = ManuallyDrop::into_inner(w); p as u32 }, E194::D0(..) => 9001, E194::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E195 { D1((), Infallible), D0(&'static Void, Void), Live(Result<NonZeroU8, Void>), D2((Never, Never)), U0, U1 }
suite!(195, s195, E195, [E195::Live(Ok(NonZeroU8::new(1).unwrap())), E195::Live(Ok(NonZeroU8::new(255).unwrap())), E195::U0, E195::U1], [E195::Live(Ok(black_box(NonZeroU8::new(1).unwrap()))), E195::Live(Ok(black_box(NonZeroU8::new(255).unwrap())))], |e| match e { E195::D1(..) => 9002, E195::D0(..) => 9001, E195::Live(w) => { let p: NonZeroU8 = (match w { Ok(x) => x, Err(_) => loop {} }); p.get() as u32 }, E195::D2(..) => 9003, E195::U0 => 1001, E195::U1 => 1002 });
#[derive(Clone, Copy)] enum E196 { U1, Live(Tr<Sg2>), D0(Wrap<HalfDead>, ()), U0 }
suite!(196, s196, E196, [E196::U1, E196::Live(Tr(Sg2::A)), E196::Live(Tr(Sg2::B)), E196::D0(Wrap(HalfDead::B), ()), E196::U0], [E196::Live(Tr(black_box(Sg2::A))), E196::Live(Tr(black_box(Sg2::B)))], |e| match e { E196::U1 => 1002, E196::Live(w) => { let p: Sg2 = w.0; p as i32 as u32 }, E196::D0(..) => 2001, E196::U0 => 1001 });
#[derive(Clone, Copy)] enum E197 { U0, D0(Void, (), PhantomData<u8>), Live(Named<NonZeroU64>), U1 }
suite!(197, s197, E197, [E197::U0, E197::Live(Named { z: (), v: NonZeroU64::new(1).unwrap(), p: PhantomData }), E197::Live(Named { z: (), v: NonZeroU64::new(0xFFFF_FFFF_FFFF_FFFF).unwrap(), p: PhantomData }), E197::Live(Named { z: (), v: NonZeroU64::new(0x1_0000_0000).unwrap(), p: PhantomData }), E197::U1], [E197::Live(Named { z: (), v: black_box(NonZeroU64::new(1).unwrap()), p: PhantomData }), E197::Live(Named { z: (), v: black_box(NonZeroU64::new(0xFFFF_FFFF_FFFF_FFFF).unwrap()), p: PhantomData }), E197::Live(Named { z: (), v: black_box(NonZeroU64::new(0x1_0000_0000).unwrap()), p: PhantomData })], |e| match e { E197::U0 => 1001, E197::D0(..) => 9001, E197::Live(w) => { let p: NonZeroU64 = w.v; (p.get() % 1_000_003) as u32 }, E197::U1 => 1002 });
#[derive(Clone, Copy)] enum E198 { D1([u32; 0], Void), U0, Live(Ordering), D0(Void, [u64; 0]) }
suite!(198, s198, E198, [E198::U0, E198::Live(Ordering::Less), E198::Live(Ordering::Equal), E198::Live(Ordering::Greater)], [E198::Live(black_box(Ordering::Less)), E198::Live(black_box(Ordering::Equal)), E198::Live(black_box(Ordering::Greater))], |e| match e { E198::D1(..) => 9002, E198::U0 => 1001, E198::Live(w) => { let p: Ordering = w; p as i32 as u32 }, E198::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E199 { Live(Result<bool, Void>), D0(PhantomData<Void>), D1(Option<Void>), U1, U0 }
suite!(199, s199, E199, [E199::Live(Ok(true)), E199::Live(Ok(false)), E199::D0(PhantomData), E199::D1(None), E199::U1, E199::U0], [E199::Live(Ok(black_box(true))), E199::Live(Ok(black_box(false)))], |e| match e { E199::Live(w) => { let p: bool = (match w { Ok(x) => x, Err(_) => loop {} }); 10 + p as u32 }, E199::D0(..) => 2001, E199::D1(..) => 2002, E199::U1 => 1002, E199::U0 => 1001 });

#[unsafe(no_mangle)]
pub fn rust_entry() {
    s180(); s181(); s182(); s183(); s184(); s185(); s186(); s187(); s188(); s189(); s190(); s191(); s192(); s193(); s194(); s195(); s196(); s197(); s198(); s199();
}
