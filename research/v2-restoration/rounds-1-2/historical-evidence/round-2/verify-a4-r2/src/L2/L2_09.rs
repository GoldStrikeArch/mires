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
#[derive(Clone, Copy)] enum E180 { U0, Live(Named<NonZeroU8>), D0(ManuallyDrop<Void>), D1(Wrap<Void>), D2(&'static Void, Void), U1 }
suite!(180, s180, E180, [E180::U0, E180::Live(Named { z: (), v: NonZeroU8::new(1).unwrap(), p: PhantomData }), E180::Live(Named { z: (), v: NonZeroU8::new(255).unwrap(), p: PhantomData }), E180::U1], |e| match e { E180::U0 => 1001, E180::Live(w) => { let p: NonZeroU8 = w.v; p.get() as u32 }, E180::D0(..) => 9001, E180::D1(..) => 9002, E180::D2(..) => 9003, E180::U1 => 1002 });
#[derive(Clone, Copy)] enum E181 { Live(Tr<S16>), U0, U1, D0([u32; 0]) }
suite!(181, s181, E181, [E181::Live(Tr(S16::A)), E181::Live(Tr(S16::B)), E181::U0, E181::U1, E181::D0([])], |e| match e { E181::Live(w) => { let p: S16 = w.0; p as i32 as u32 }, E181::U0 => 1001, E181::U1 => 1002, E181::D0(..) => 2001 });
#[derive(Clone, Copy)] enum E182 { U1, D0(MaybeUninit<Void>), U5, Live(Wrap<fn() -> u32>), U0, U4, U3, U2 }
suite!(182, s182, E182, [E182::U1, E182::U5, E182::Live(Wrap(seven)), E182::Live(Wrap(eight)), E182::U0, E182::U4, E182::U3, E182::U2], |e| match e { E182::U1 => 1002, E182::D0(..) => 9001, E182::U5 => 1006, E182::Live(w) => { let p: fn() -> u32 = w.0; p() }, E182::U0 => 1001, E182::U4 => 1005, E182::U3 => 1004, E182::U2 => 1003 });
#[derive(Clone, Copy)] enum E183 { Live(Result<NonZeroU16, Void>), D0(Aligned) }
suite!(183, s183, E183, [E183::Live(Ok(NonZeroU16::new(1).unwrap())), E183::Live(Ok(NonZeroU16::new(65535).unwrap()))], |e| match e { E183::Live(w) => { let p: NonZeroU16 = (match w { Ok(x) => x, Err(_) => loop {} }); p.get() as u32 }, E183::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E184 { D0(NamedDead), D1(Result<(), Void>), U0, U3, U4, D2(TrDead), Live((W16, PhantomData<Void>, [Void; 0])), U2, U5, U1 }
suite!(184, s184, E184, [E184::D1(Ok(())), E184::U0, E184::U3, E184::U4, E184::Live((W16::A, PhantomData, [])), E184::Live((W16::B, PhantomData, [])), E184::U2, E184::U5, E184::U1], |e| match e { E184::D0(..) => 9001, E184::D1(..) => 2002, E184::U0 => 1001, E184::U3 => 1004, E184::U4 => 1005, E184::D2(..) => 9003, E184::Live(w) => { let p: W16 = w.0; p as u32 }, E184::U2 => 1003, E184::U5 => 1006, E184::U1 => 1002 });
#[derive(Clone, Copy)] enum E185 { D0(AllDead), D1(PkDead), D2((Void,)), Live((W16,)) }
suite!(185, s185, E185, [E185::Live((W16::A,)), E185::Live((W16::B,))], |e| match e { E185::D0(..) => 9001, E185::D1(..) => 9002, E185::D2(..) => 9003, E185::Live(w) => { let p: W16 = w.0; p as u32 } });
#[derive(Clone, Copy)] enum E186 { U1, U5, D0((), Infallible), U3, U4, Live(SL3<&'static u32>), U0, U2 }
suite!(186, s186, E186, [E186::U1, E186::U5, E186::U3, E186::U4, E186::Live(SL3::Live(&5)), E186::Live(SL3::Live(&SEVEN)), E186::U0, E186::U2], |e| match e { E186::U1 => 1002, E186::U5 => 1006, E186::D0(..) => 9001, E186::U3 => 1004, E186::U4 => 1005, E186::Live(w) => { let p: &'static u32 = (match w { SL3::Live(x) => x, _ => loop {} }); *p }, E186::U0 => 1001, E186::U2 => 1003 });
#[derive(Clone, Copy)] enum E187 { U2, U1, D2(DeepDead), U0, D1(Wrap<Void>), Live(Wrap<Tr<((), Sg2)>>), D0(CDead) }
suite!(187, s187, E187, [E187::U2, E187::U1, E187::U0, E187::Live(Wrap(Tr(((), Sg2::A)))), E187::Live(Wrap(Tr(((), Sg2::B))))], |e| match e { E187::U2 => 1003, E187::U1 => 1002, E187::D2(..) => 9003, E187::U0 => 1001, E187::D1(..) => 9002, E187::Live(w) => { let p: Sg2 = w.0.0.1; p as i32 as u32 }, E187::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E188 { U1, U0, Live(ManuallyDrop<S16>) }
suite!(188, s188, E188, [E188::U1, E188::U0, E188::Live(ManuallyDrop::new(S16::A)), E188::Live(ManuallyDrop::new(S16::B))], |e| match e { E188::U1 => 1002, E188::U0 => 1001, E188::Live(w) => { let p: S16 = ManuallyDrop::into_inner(w); p as i32 as u32 } });
#[derive(Clone, Copy)] enum E189 { D0(Void), Live(SL<Wrap<SL3<NonZeroU32>>>) }
suite!(189, s189, E189, [E189::Live(SL::Live(Wrap(SL3::Live(NonZeroU32::new(1).unwrap())))), E189::Live(SL::Live(Wrap(SL3::Live(NonZeroU32::new(0xFFFF_FFFF).unwrap()))))], |e| match e { E189::D0(..) => 9001, E189::Live(w) => { let p: NonZeroU32 = (match w { SL::Live(Wrap(SL3::Live(x))) => x, _ => loop {} }); p.get() } });
#[derive(Clone, Copy)] enum E190 { U1, U0, U2, D1([Void; 2]), U3, D0(NamedDead), Live(Wrap<WU>) }
suite!(190, s190, E190, [E190::U1, E190::U0, E190::U2, E190::U3, E190::Live(Wrap(WU::A)), E190::Live(Wrap(WU::B))], |e| match e { E190::U1 => 1002, E190::U0 => 1001, E190::U2 => 1003, E190::D1(..) => 9002, E190::U3 => 1004, E190::D0(..) => 9001, E190::Live(w) => { let p: WU = w.0; p as usize as u32 } });
#[derive(Clone, Copy)] enum E191 { D0(Wrap<Void>), Live(ZPad<Option<char>>) }
suite!(191, s191, E191, [E191::Live(ZPad([], Some('z'), ())), E191::Live(ZPad([], None, ()))], |e| match e { E191::D0(..) => 9001, E191::Live(w) => { let p: Option<char> = w.1; match p { Some(c) => c as u32, None => 48 } } });
#[derive(Clone, Copy)] enum E192 { Live(ZPad<Sg2>), U3, U2, U0, U1 }
suite!(192, s192, E192, [E192::Live(ZPad([], Sg2::A, ())), E192::Live(ZPad([], Sg2::B, ())), E192::U3, E192::U2, E192::U0, E192::U1], |e| match e { E192::Live(w) => { let p: Sg2 = w.1; p as i32 as u32 }, E192::U3 => 1004, E192::U2 => 1003, E192::U0 => 1001, E192::U1 => 1002 });
#[derive(Clone, Copy)] enum E193 { Live(Wrap<Tr<((), NonZeroU64)>>), D0((), Infallible), U0 }
suite!(193, s193, E193, [E193::Live(Wrap(Tr(((), NonZeroU64::new(1).unwrap())))), E193::Live(Wrap(Tr(((), NonZeroU64::new(0xFFFF_FFFF_FFFF_FFFF).unwrap())))), E193::Live(Wrap(Tr(((), NonZeroU64::new(0x1_0000_0000).unwrap())))), E193::U0], |e| match e { E193::Live(w) => { let p: NonZeroU64 = w.0.0.1; (p.get() % 1_000_003) as u32 }, E193::D0(..) => 9001, E193::U0 => 1001 });
#[derive(Clone, Copy)] enum E194 { D1(Wrap<HalfDead>, ()), U1, U0, D2(NamedDead), D0(HalfDead2), Live([W16; 1]) }
suite!(194, s194, E194, [E194::D1(Wrap(HalfDead::B), ()), E194::U1, E194::U0, E194::D0(HalfDead2::A), E194::Live([W16::A]), E194::Live([W16::B])], |e| match e { E194::D1(..) => 2002, E194::U1 => 1002, E194::U0 => 1001, E194::D2(..) => 9003, E194::D0(..) => 2001, E194::Live(w) => { let p: W16 = w[0]; p as u32 } });
#[derive(Clone, Copy)] enum E195 { D1([u16; 0], Void), Live(ManuallyDrop<Sg2>), D0(PkDead), D2(ManuallyDrop<Void>) }
suite!(195, s195, E195, [E195::Live(ManuallyDrop::new(Sg2::A)), E195::Live(ManuallyDrop::new(Sg2::B))], |e| match e { E195::D1(..) => 9002, E195::Live(w) => { let p: Sg2 = ManuallyDrop::into_inner(w); p as i32 as u32 }, E195::D0(..) => 9001, E195::D2(..) => 9003 });
#[derive(Clone, Copy)] enum E196 { D1(CDead), D0(HalfDead), Live(SL<Two>), D2(Void, (), PhantomData<u8>), U0 }
suite!(196, s196, E196, [E196::D0(HalfDead::B), E196::Live(SL::Live(Two::A)), E196::Live(SL::Live(Two::B)), E196::U0], |e| match e { E196::D1(..) => 9002, E196::D0(..) => 2001, E196::Live(w) => { let p: Two = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); p as u32 + 30 }, E196::D2(..) => 9003, E196::U0 => 1001 });
#[derive(Clone, Copy)] enum E197 { D2((Void,)), Live(SL<Sg>), D1(AllDead), D0(Infallible) }
suite!(197, s197, E197, [E197::Live(SL::Live(Sg::A)), E197::Live(SL::Live(Sg::B))], |e| match e { E197::D2(..) => 9003, E197::Live(w) => { let p: Sg = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); p as i32 as u32 }, E197::D1(..) => 9002, E197::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E198 { U0, U3, U4, U5, Live(ZPad<fn() -> u32>), D0((Void, ())), U2, U1 }
suite!(198, s198, E198, [E198::U0, E198::U3, E198::U4, E198::U5, E198::Live(ZPad([], seven, ())), E198::Live(ZPad([], eight, ())), E198::U2, E198::U1], |e| match e { E198::U0 => 1001, E198::U3 => 1004, E198::U4 => 1005, E198::U5 => 1006, E198::Live(w) => { let p: fn() -> u32 = w.1; p() }, E198::D0(..) => 9001, E198::U2 => 1003, E198::U1 => 1002 });
#[derive(Clone, Copy)] enum E199 { U0, Live(((), Sg2)) }
suite!(199, s199, E199, [E199::U0, E199::Live(((), Sg2::A)), E199::Live(((), Sg2::B))], |e| match e { E199::U0 => 1001, E199::Live(w) => { let p: Sg2 = w.1; p as i32 as u32 } });

#[unsafe(no_mangle)]
pub fn rust_entry() {
    s180(); s181(); s182(); s183(); s184(); s185(); s186(); s187(); s188(); s189(); s190(); s191(); s192(); s193(); s194(); s195(); s196(); s197(); s198(); s199();
}
