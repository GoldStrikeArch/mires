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
#[derive(Clone, Copy)] enum E260 { Live(W64), D0(Result<(), Void>), U1, U0 }
suite!(260, s260, E260, [E260::Live(W64::A), E260::Live(W64::B), E260::D0(Ok(())), E260::U1, E260::U0], [E260::Live(black_box(W64::A)), E260::Live(black_box(W64::B))], |e| match e { E260::Live(w) => { let p: W64 = w; (p as u64 % 1_000_003) as u32 }, E260::D0(..) => 2001, E260::U1 => 1002, E260::U0 => 1001 });
#[derive(Clone, Copy)] enum E261 { U0, D1(Aligned), Live(Result<S32, Void>), U1, D0(HalfDead2) }
suite!(261, s261, E261, [E261::U0, E261::Live(Ok(S32::A)), E261::Live(Ok(S32::B)), E261::U1, E261::D0(HalfDead2::A)], [E261::Live(Ok(black_box(S32::A))), E261::Live(Ok(black_box(S32::B)))], |e| match e { E261::U0 => 1001, E261::D1(..) => 9002, E261::Live(w) => { let p: S32 = (match w { Ok(x) => x, Err(_) => loop {} }); p as i32 as u32 }, E261::U1 => 1002, E261::D0(..) => 2001 });
#[derive(Clone, Copy)] enum E262 { U5, U2, U3, U4, U0, Live((bool, PhantomData<Void>, [Void; 0])), U1 }
suite!(262, s262, E262, [E262::U5, E262::U2, E262::U3, E262::U4, E262::U0, E262::Live((true, PhantomData, [])), E262::Live((false, PhantomData, [])), E262::U1], [E262::Live((black_box(true), PhantomData, [])), E262::Live((black_box(false), PhantomData, []))], |e| match e { E262::U5 => 1006, E262::U2 => 1003, E262::U3 => 1004, E262::U4 => 1005, E262::U0 => 1001, E262::Live(w) => { let p: bool = w.0; 10 + p as u32 }, E262::U1 => 1002 });
#[derive(Clone, Copy)] enum E263 { U0, Live(Wrap<Hi2>), D0(&'static Void, Void) }
suite!(263, s263, E263, [E263::U0, E263::Live(Wrap(Hi2::A)), E263::Live(Wrap(Hi2::B))], [E263::Live(Wrap(black_box(Hi2::A))), E263::Live(Wrap(black_box(Hi2::B)))], |e| match e { E263::U0 => 1001, E263::Live(w) => { let p: Hi2 = w.0; p as u32 }, E263::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E264 { U2, U4, U5, U1, U0, U3, Live(Result<Never, Three>) }
suite!(264, s264, E264, [E264::U2, E264::U4, E264::U5, E264::U1, E264::U0, E264::U3, E264::Live(Err(Three::A)), E264::Live(Err(Three::B)), E264::Live(Err(Three::C))], [E264::Live(Err(black_box(Three::A))), E264::Live(Err(black_box(Three::B))), E264::Live(Err(black_box(Three::C)))], |e| match e { E264::U2 => 1003, E264::U4 => 1005, E264::U5 => 1006, E264::U1 => 1002, E264::U0 => 1001, E264::U3 => 1004, E264::Live(w) => { let p: Three = (match w { Err(x) => x, Ok(_) => loop {} }); p as u32 + 20 } });
#[derive(Clone, Copy)] enum E265 { U1, U0, Live(Named<W32>), D0((HalfDead, Void)) }
suite!(265, s265, E265, [E265::U1, E265::U0, E265::Live(Named { z: (), v: W32::A, p: PhantomData }), E265::Live(Named { z: (), v: W32::B, p: PhantomData })], [E265::Live(Named { z: (), v: black_box(W32::A), p: PhantomData }), E265::Live(Named { z: (), v: black_box(W32::B), p: PhantomData })], |e| match e { E265::U1 => 1002, E265::U0 => 1001, E265::Live(w) => { let p: W32 = w.v; p as u32 }, E265::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E266 { D1([u32; 0]), Live([WU; 1]), D0([(Void,); 0]) }
suite!(266, s266, E266, [E266::D1([]), E266::Live([WU::A]), E266::Live([WU::B]), E266::D0([])], [E266::Live([black_box(WU::A)]), E266::Live([black_box(WU::B)])], |e| match e { E266::D1(..) => 2002, E266::Live(w) => { let p: WU = w[0]; p as usize as u32 }, E266::D0(..) => 2001 });
#[derive(Clone, Copy)] enum E267 { U0, D0((Void, ())), U2, Live(ZPad<NonZeroU8>), U1 }
suite!(267, s267, E267, [E267::U0, E267::U2, E267::Live(ZPad([], NonZeroU8::new(1).unwrap(), ())), E267::Live(ZPad([], NonZeroU8::new(255).unwrap(), ())), E267::U1], [E267::Live(ZPad([], black_box(NonZeroU8::new(1).unwrap()), ())), E267::Live(ZPad([], black_box(NonZeroU8::new(255).unwrap()), ()))], |e| match e { E267::U0 => 1001, E267::D0(..) => 9001, E267::U2 => 1003, E267::Live(w) => { let p: NonZeroU8 = w.1; p.get() as u32 }, E267::U1 => 1002 });
#[derive(Clone, Copy)] enum E268 { U1, D1((Never, Never)), D0(u8, Void), D2((HalfDead, Void)), Live(Sg2), U0, U2 }
suite!(268, s268, E268, [E268::U1, E268::Live(Sg2::A), E268::Live(Sg2::B), E268::U0, E268::U2], [E268::Live(black_box(Sg2::A)), E268::Live(black_box(Sg2::B))], |e| match e { E268::U1 => 1002, E268::D1(..) => 9002, E268::D0(..) => 9001, E268::D2(..) => 9003, E268::Live(w) => { let p: Sg2 = w; p as i32 as u32 }, E268::U0 => 1001, E268::U2 => 1003 });
#[derive(Clone, Copy)] enum E269 { D1(Result<Void, Never>), D2((Never, Never)), U2, U1, D0(([Void; 0], Void)), U0, Live((Option<Hi2>, PhantomData<Void>, [Void; 0])) }
suite!(269, s269, E269, [E269::U2, E269::U1, E269::U0, E269::Live((Some(Hi2::A), PhantomData, [])), E269::Live((Some(Hi2::B), PhantomData, [])), E269::Live((None, PhantomData, []))], [E269::Live((black_box(Some(Hi2::A)), PhantomData, [])), E269::Live((black_box(Some(Hi2::B)), PhantomData, [])), E269::Live((black_box(None), PhantomData, []))], |e| match e { E269::D1(..) => 9002, E269::D2(..) => 9003, E269::U2 => 1003, E269::U1 => 1002, E269::D0(..) => 9001, E269::U0 => 1001, E269::Live(w) => { let p: Option<Hi2> = w.0; match p { Some(h) => h as u32, None => 47 } } });
#[derive(Clone, Copy)] enum E270 { D0(NamedDead), Live(SL<Wrap<SL3<Hi>>>), U0 }
suite!(270, s270, E270, [E270::Live(SL::Live(Wrap(SL3::Live(Hi::A)))), E270::Live(SL::Live(Wrap(SL3::Live(Hi::B)))), E270::U0], [E270::Live(SL::Live(Wrap(SL3::Live(black_box(Hi::A))))), E270::Live(SL::Live(Wrap(SL3::Live(black_box(Hi::B)))))], |e| match e { E270::D0(..) => 9001, E270::Live(w) => { let p: Hi = (match w { SL::Live(Wrap(SL3::Live(x))) => x, _ => loop {} }); p as u32 }, E270::U0 => 1001 });
#[derive(Clone, Copy)] enum E271 { Live(CW<S64>), U3, U1, U2, D0(Void, [u64; 0]), U0 }
suite!(271, s271, E271, [E271::Live(CW(S64::A)), E271::Live(CW(S64::B)), E271::U3, E271::U1, E271::U2, E271::U0], [E271::Live(CW(black_box(S64::A))), E271::Live(CW(black_box(S64::B)))], |e| match e { E271::Live(w) => { let p: S64 = w.0; (p as i64 as u64 % 1_000_003) as u32 }, E271::U3 => 1004, E271::U1 => 1002, E271::U2 => 1003, E271::D0(..) => 9001, E271::U0 => 1001 });
#[derive(Clone, Copy)] enum E272 { U0, U1, Live([Ordering; 1]), D0(Result<Void, Never>), D1([u32; 0]) }
suite!(272, s272, E272, [E272::U0, E272::U1, E272::Live([Ordering::Less]), E272::Live([Ordering::Equal]), E272::Live([Ordering::Greater]), E272::D1([])], [E272::Live([black_box(Ordering::Less)]), E272::Live([black_box(Ordering::Equal)]), E272::Live([black_box(Ordering::Greater)])], |e| match e { E272::U0 => 1001, E272::U1 => 1002, E272::Live(w) => { let p: Ordering = w[0]; p as i32 as u32 }, E272::D0(..) => 9001, E272::D1(..) => 2002 });
#[derive(Clone, Copy)] enum E273 { Live(Pk<NonZeroI8>), D0([u32; 0]), U0, U1 }
suite!(273, s273, E273, [E273::Live(Pk(NonZeroI8::new(-1).unwrap())), E273::Live(Pk(NonZeroI8::new(127).unwrap())), E273::D0([]), E273::U0, E273::U1], [E273::Live(Pk(black_box(NonZeroI8::new(-1).unwrap()))), E273::Live(Pk(black_box(NonZeroI8::new(127).unwrap())))], |e| match e { E273::Live(w) => { let p: NonZeroI8 = { w.0 }; p.get() as i32 as u32 }, E273::D0(..) => 2001, E273::U0 => 1001, E273::U1 => 1002 });
#[derive(Clone, Copy)] enum E274 { D1(MaybeUninit<Void>), D2(CDead), U0, Live(SL<W32>), D0(Aligned) }
suite!(274, s274, E274, [E274::U0, E274::Live(SL::Live(W32::A)), E274::Live(SL::Live(W32::B))], [E274::Live(SL::Live(black_box(W32::A))), E274::Live(SL::Live(black_box(W32::B)))], |e| match e { E274::D1(..) => 9002, E274::D2(..) => 9003, E274::U0 => 1001, E274::Live(w) => { let p: W32 = (match w { SL::Live(x) => x, SL::Never(..) => loop {} }); p as u32 }, E274::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E275 { Live([NonZeroU32; 1]), D0([u16; 0], Void), D1(NamedDead), D2([u32; 0]), U0 }
suite!(275, s275, E275, [E275::Live([NonZeroU32::new(1).unwrap()]), E275::Live([NonZeroU32::new(0xFFFF_FFFF).unwrap()]), E275::D2([]), E275::U0], [E275::Live([black_box(NonZeroU32::new(1).unwrap())]), E275::Live([black_box(NonZeroU32::new(0xFFFF_FFFF).unwrap())])], |e| match e { E275::Live(w) => { let p: NonZeroU32 = w[0]; p.get() }, E275::D0(..) => 9001, E275::D1(..) => 9002, E275::D2(..) => 2003, E275::U0 => 1001 });
#[derive(Clone, Copy)] enum E276 { D0(Wrap<HalfDead>, ()), Live(Wrap<Option<char>>), D1(Result<(), Void>) }
suite!(276, s276, E276, [E276::D0(Wrap(HalfDead::B), ()), E276::Live(Wrap(Some('z'))), E276::Live(Wrap(None)), E276::D1(Ok(()))], [E276::Live(Wrap(black_box(Some('z')))), E276::Live(Wrap(black_box(None)))], |e| match e { E276::D0(..) => 2001, E276::Live(w) => { let p: Option<char> = w.0; match p { Some(c) => c as u32, None => 48 } }, E276::D1(..) => 2002 });
#[derive(Clone, Copy)] enum E277 { U4, Live(Sg2), U2, U5, U0, U1, D0(&'static Void, Void), D1(Result<Void, Never>), U3 }
suite!(277, s277, E277, [E277::U4, E277::Live(Sg2::A), E277::Live(Sg2::B), E277::U2, E277::U5, E277::U0, E277::U1, E277::U3], [E277::Live(black_box(Sg2::A)), E277::Live(black_box(Sg2::B))], |e| match e { E277::U4 => 1005, E277::Live(w) => { let p: Sg2 = w; p as i32 as u32 }, E277::U2 => 1003, E277::U5 => 1006, E277::U0 => 1001, E277::U1 => 1002, E277::D0(..) => 9001, E277::D1(..) => 9002, E277::U3 => 1004 });
#[derive(Clone, Copy)] enum E278 { Live(Result<Never, Small>), U0, D0(Wrap<Void>) }
suite!(278, s278, E278, [E278::Live(Err(Small::X)), E278::Live(Err(Small::Y)), E278::U0], [E278::Live(Err(black_box(Small::X))), E278::Live(Err(black_box(Small::Y)))], |e| match e { E278::Live(w) => { let p: Small = (match w { Err(x) => x, Ok(_) => loop {} }); p as u32 }, E278::U0 => 1001, E278::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E279 { Live((NonZeroU32, PhantomData<Void>, [Void; 0])), D0((Void,)), D1(HalfDead2) }
suite!(279, s279, E279, [E279::Live((NonZeroU32::new(1).unwrap(), PhantomData, [])), E279::Live((NonZeroU32::new(0xFFFF_FFFF).unwrap(), PhantomData, [])), E279::D1(HalfDead2::A)], [E279::Live((black_box(NonZeroU32::new(1).unwrap()), PhantomData, [])), E279::Live((black_box(NonZeroU32::new(0xFFFF_FFFF).unwrap()), PhantomData, []))], |e| match e { E279::Live(w) => { let p: NonZeroU32 = w.0; p.get() }, E279::D0(..) => 9001, E279::D1(..) => 2002 });

#[unsafe(no_mangle)]
pub fn rust_entry() {
    s260(); s261(); s262(); s263(); s264(); s265(); s266(); s267(); s268(); s269(); s270(); s271(); s272(); s273(); s274(); s275(); s276(); s277(); s278(); s279();
}
