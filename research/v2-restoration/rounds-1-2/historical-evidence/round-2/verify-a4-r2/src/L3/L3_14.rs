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
#[derive(Clone, Copy)] enum E280 { Live(SL<Sg2>), D1(Wrap<AllDead>), D0(ManuallyDrop<Void>) }
suite!(280, s280, E280, [E280::Live(SL::Live(Sg2::A)), E280::Live(SL::Live(Sg2::B))], [E280::Live(SL::Live(black_box(Sg2::A))), E280::Live(SL::Live(black_box(Sg2::B)))], |e| match e { E280::Live(w) => { let p: Sg2 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); p as i32 as u32 }, E280::D1(..) => 9002, E280::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E281 { U0, U1, Live(Wrap<Tr<((), Hi3)>>), U2 }
suite!(281, s281, E281, [E281::U0, E281::U1, E281::Live(Wrap(Tr(((), Hi3::A)))), E281::Live(Wrap(Tr(((), Hi3::B)))), E281::Live(Wrap(Tr(((), Hi3::C)))), E281::U2], [E281::Live(Wrap(Tr(((), black_box(Hi3::A))))), E281::Live(Wrap(Tr(((), black_box(Hi3::B))))), E281::Live(Wrap(Tr(((), black_box(Hi3::C)))))], |e| match e { E281::U0 => 1001, E281::U1 => 1002, E281::Live(w) => { let p: Hi3 = w.0.0.1; p as u32 }, E281::U2 => 1003 });
#[derive(Clone, Copy)] enum E282 { D0(CDead), U2, D1(UDead), D2(NamedDead), Live(ZPad<WU>), U3, U1, U0 }
suite!(282, s282, E282, [E282::U2, E282::Live(ZPad([], WU::A, ())), E282::Live(ZPad([], WU::B, ())), E282::U3, E282::U1, E282::U0], [E282::Live(ZPad([], black_box(WU::A), ())), E282::Live(ZPad([], black_box(WU::B), ()))], |e| match e { E282::D0(..) => 9001, E282::U2 => 1003, E282::D1(..) => 9002, E282::D2(..) => 9003, E282::Live(w) => { let p: WU = w.1; p as usize as u32 }, E282::U3 => 1004, E282::U1 => 1002, E282::U0 => 1001 });
#[derive(Clone, Copy)] enum E283 { U0, U2, U1, Live((S32,)) }
suite!(283, s283, E283, [E283::U0, E283::U2, E283::U1, E283::Live((S32::A,)), E283::Live((S32::B,))], [E283::Live((black_box(S32::A),)), E283::Live((black_box(S32::B),))], |e| match e { E283::U0 => 1001, E283::U2 => 1003, E283::U1 => 1002, E283::Live(w) => { let p: S32 = w.0; p as i32 as u32 } });
#[derive(Clone, Copy)] enum E284 { D0(HalfDead), D2(Wrap<Void>), D1(Void, [u64; 0]), U0, Live(Pk<NonZeroU16>) }
suite!(284, s284, E284, [E284::D0(HalfDead::B), E284::U0, E284::Live(Pk(NonZeroU16::new(1).unwrap())), E284::Live(Pk(NonZeroU16::new(65535).unwrap()))], [E284::Live(Pk(black_box(NonZeroU16::new(1).unwrap()))), E284::Live(Pk(black_box(NonZeroU16::new(65535).unwrap())))], |e| match e { E284::D0(..) => 2001, E284::D2(..) => 9003, E284::D1(..) => 9002, E284::U0 => 1001, E284::Live(w) => { let p: NonZeroU16 = { w.0 }; p.get() as u32 } });
#[derive(Clone, Copy)] enum E285 { U2, U0, U3, U1, Live(Result<Never, Sg>) }
suite!(285, s285, E285, [E285::U2, E285::U0, E285::U3, E285::U1, E285::Live(Err(Sg::A)), E285::Live(Err(Sg::B))], [E285::Live(Err(black_box(Sg::A))), E285::Live(Err(black_box(Sg::B)))], |e| match e { E285::U2 => 1003, E285::U0 => 1001, E285::U3 => 1004, E285::U1 => 1002, E285::Live(w) => { let p: Sg = (match w { Err(x) => x, Ok(_) => loop {} }); p as i32 as u32 } });
#[derive(Clone, Copy)] enum E286 { U0, Live(SL<&'static u32>) }
suite!(286, s286, E286, [E286::U0, E286::Live(SL::Live(&5)), E286::Live(SL::Live(&SEVEN))], [E286::Live(SL::Live(black_box(&5))), E286::Live(SL::Live(black_box(&SEVEN)))], |e| match e { E286::U0 => 1001, E286::Live(w) => { let p: &'static u32 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); *p } });
#[derive(Clone, Copy)] enum E287 { U5, U4, D2(PhantomData<Void>), D0(Wrap<HalfDead>, ()), Live(NonZeroU8), U0, D1([u32; 0], Void), U1, U2, U3 }
suite!(287, s287, E287, [E287::U5, E287::U4, E287::D2(PhantomData), E287::D0(Wrap(HalfDead::B), ()), E287::Live(NonZeroU8::new(1).unwrap()), E287::Live(NonZeroU8::new(255).unwrap()), E287::U0, E287::U1, E287::U2, E287::U3], [E287::Live(black_box(NonZeroU8::new(1).unwrap())), E287::Live(black_box(NonZeroU8::new(255).unwrap()))], |e| match e { E287::U5 => 1006, E287::U4 => 1005, E287::D2(..) => 2003, E287::D0(..) => 2001, E287::Live(w) => { let p: NonZeroU8 = w; p.get() as u32 }, E287::U0 => 1001, E287::D1(..) => 9002, E287::U1 => 1002, E287::U2 => 1003, E287::U3 => 1004 });
#[derive(Clone, Copy)] enum E288 { D2(DeepDead), Live(Named<NonZeroU16>), U0, D0([(Void,); 0]), D1(ManuallyDrop<Void>) }
suite!(288, s288, E288, [E288::Live(Named { z: (), v: NonZeroU16::new(1).unwrap(), p: PhantomData }), E288::Live(Named { z: (), v: NonZeroU16::new(65535).unwrap(), p: PhantomData }), E288::U0, E288::D0([])], [E288::Live(Named { z: (), v: black_box(NonZeroU16::new(1).unwrap()), p: PhantomData }), E288::Live(Named { z: (), v: black_box(NonZeroU16::new(65535).unwrap()), p: PhantomData })], |e| match e { E288::D2(..) => 9003, E288::Live(w) => { let p: NonZeroU16 = w.v; p.get() as u32 }, E288::U0 => 1001, E288::D0(..) => 2001, E288::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E289 { D2([u16; 0], Void), D1((), Infallible), Live((fn() -> u32, PhantomData<Void>, [Void; 0])), D0(NamedDead) }
suite!(289, s289, E289, [E289::Live((seven, PhantomData, [])), E289::Live((eight, PhantomData, []))], [E289::Live((black_box(seven), PhantomData, [])), E289::Live((black_box(eight), PhantomData, []))], |e| match e { E289::D2(..) => 9003, E289::D1(..) => 9002, E289::Live(w) => { let p: fn() -> u32 = w.0; p() }, E289::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E290 { Live(Wrap<Option<char>>), U0, D0(AllDead) }
suite!(290, s290, E290, [E290::Live(Wrap(Some('z'))), E290::Live(Wrap(None)), E290::U0], [E290::Live(Wrap(black_box(Some('z')))), E290::Live(Wrap(black_box(None)))], |e| match e { E290::Live(w) => { let p: Option<char> = w.0; match p { Some(c) => c as u32, None => 48 } }, E290::U0 => 1001, E290::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E291 { D0([u16; 0], Void), Live(SL<W32>) }
suite!(291, s291, E291, [E291::Live(SL::Live(W32::A)), E291::Live(SL::Live(W32::B))], [E291::Live(SL::Live(black_box(W32::A))), E291::Live(SL::Live(black_box(W32::B)))], |e| match e { E291::D0(..) => 9001, E291::Live(w) => { let p: W32 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); p as u32 } });
#[derive(Clone, Copy)] enum E292 { U1, U3, D0((Void, ())), U0, U2, Live(Result<Never, NonZeroU64>) }
suite!(292, s292, E292, [E292::U1, E292::U3, E292::U0, E292::U2, E292::Live(Err(NonZeroU64::new(1).unwrap())), E292::Live(Err(NonZeroU64::new(0xFFFF_FFFF_FFFF_FFFF).unwrap())), E292::Live(Err(NonZeroU64::new(0x1_0000_0000).unwrap()))], [E292::Live(Err(black_box(NonZeroU64::new(1).unwrap()))), E292::Live(Err(black_box(NonZeroU64::new(0xFFFF_FFFF_FFFF_FFFF).unwrap()))), E292::Live(Err(black_box(NonZeroU64::new(0x1_0000_0000).unwrap())))], |e| match e { E292::U1 => 1002, E292::U3 => 1004, E292::D0(..) => 9001, E292::U0 => 1001, E292::U2 => 1003, E292::Live(w) => { let p: NonZeroU64 = (match w { Err(x) => x, Ok(_) => loop {} }); (p.get() % 1_000_003) as u32 } });
#[derive(Clone, Copy)] enum E293 { U0, U1, Live(SL3<NonZeroU16>), U2 }
suite!(293, s293, E293, [E293::U0, E293::U1, E293::Live(SL3::Live(NonZeroU16::new(1).unwrap())), E293::Live(SL3::Live(NonZeroU16::new(65535).unwrap())), E293::U2], [E293::Live(SL3::Live(black_box(NonZeroU16::new(1).unwrap()))), E293::Live(SL3::Live(black_box(NonZeroU16::new(65535).unwrap())))], |e| match e { E293::U0 => 1001, E293::U1 => 1002, E293::Live(w) => { let p: NonZeroU16 = (match w { SL3::Live(x) => x, _ => loop {} }); p.get() as u32 }, E293::U2 => 1003 });
#[derive(Clone, Copy)] enum E294 { U0, Live(SL<W32>) }
suite!(294, s294, E294, [E294::U0, E294::Live(SL::Live(W32::A)), E294::Live(SL::Live(W32::B))], [E294::Live(SL::Live(black_box(W32::A))), E294::Live(SL::Live(black_box(W32::B)))], |e| match e { E294::U0 => 1001, E294::Live(w) => { let p: W32 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); p as u32 } });
#[derive(Clone, Copy)] enum E295 { D0(Wrap<Wrap<(Void, ())>>), Live(Result<Option<Hi2>, Void>) }
suite!(295, s295, E295, [E295::Live(Ok(Some(Hi2::A))), E295::Live(Ok(Some(Hi2::B))), E295::Live(Ok(None))], [E295::Live(Ok(black_box(Some(Hi2::A)))), E295::Live(Ok(black_box(Some(Hi2::B)))), E295::Live(Ok(black_box(None)))], |e| match e { E295::D0(..) => 9001, E295::Live(w) => { let p: Option<Hi2> = (match w { Ok(x) => x, Err(_) => loop {} }); match p { Some(h) => h as u32, None => 47 } } });
#[derive(Clone, Copy)] enum E296 { U0, D1(([Void; 0], Void)), Live(SL<Wrap<SL3<fn() -> u32>>>), U1, D0(Result<(), Void>) }
suite!(296, s296, E296, [E296::U0, E296::Live(SL::Live(Wrap(SL3::Live(seven)))), E296::Live(SL::Live(Wrap(SL3::Live(eight)))), E296::U1, E296::D0(Ok(()))], [E296::Live(SL::Live(Wrap(SL3::Live(black_box(seven))))), E296::Live(SL::Live(Wrap(SL3::Live(black_box(eight)))))], |e| match e { E296::U0 => 1001, E296::D1(..) => 9002, E296::Live(w) => { let p: fn() -> u32 = (match w { SL::Live(Wrap(SL3::Live(x))) => x, _ => loop {} }); p() }, E296::U1 => 1002, E296::D0(..) => 2001 });
#[derive(Clone, Copy)] enum E297 { Live(S32), D0(Infallible) }
suite!(297, s297, E297, [E297::Live(S32::A), E297::Live(S32::B)], [E297::Live(black_box(S32::A)), E297::Live(black_box(S32::B))], |e| match e { E297::Live(w) => { let p: S32 = w; p as i32 as u32 }, E297::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E298 { U1, U0, Live(CW<NonZeroU8>) }
suite!(298, s298, E298, [E298::U1, E298::U0, E298::Live(CW(NonZeroU8::new(1).unwrap())), E298::Live(CW(NonZeroU8::new(255).unwrap()))], [E298::Live(CW(black_box(NonZeroU8::new(1).unwrap()))), E298::Live(CW(black_box(NonZeroU8::new(255).unwrap())))], |e| match e { E298::U1 => 1002, E298::U0 => 1001, E298::Live(w) => { let p: NonZeroU8 = w.0; p.get() as u32 } });
#[derive(Clone, Copy)] enum E299 { U3, U1, U0, D2([u32; 0], Void), D0(HalfDead2), U2, D1(([Void; 0], Void)), Live(SL<Wrap<SL3<WU>>>) }
suite!(299, s299, E299, [E299::U3, E299::U1, E299::U0, E299::D0(HalfDead2::A), E299::U2, E299::Live(SL::Live(Wrap(SL3::Live(WU::A)))), E299::Live(SL::Live(Wrap(SL3::Live(WU::B))))], [E299::Live(SL::Live(Wrap(SL3::Live(black_box(WU::A))))), E299::Live(SL::Live(Wrap(SL3::Live(black_box(WU::B)))))], |e| match e { E299::U3 => 1004, E299::U1 => 1002, E299::U0 => 1001, E299::D2(..) => 9003, E299::D0(..) => 2001, E299::U2 => 1003, E299::D1(..) => 9002, E299::Live(w) => { let p: WU = (match w { SL::Live(Wrap(SL3::Live(x))) => x, _ => loop {} }); p as usize as u32 } });

#[unsafe(no_mangle)]
pub fn rust_entry() {
    s280(); s281(); s282(); s283(); s284(); s285(); s286(); s287(); s288(); s289(); s290(); s291(); s292(); s293(); s294(); s295(); s296(); s297(); s298(); s299();
}
