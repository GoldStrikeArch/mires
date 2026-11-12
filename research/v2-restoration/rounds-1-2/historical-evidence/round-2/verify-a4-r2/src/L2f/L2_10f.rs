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
#[derive(Clone, Copy)] enum E200 { D0(&'static Void, Void), Live(ZPad<Ordering>) }
suite!(200, s200, E200, [E200::Live(ZPad([], Ordering::Less, ())), E200::Live(ZPad([], Ordering::Equal, ())), E200::Live(ZPad([], Ordering::Greater, ()))], |e| match e { E200::D0(..) => 9001, E200::Live(w) => { let p: Ordering = w.1; p as i32 as u32 } });
#[derive(Clone, Copy)] enum E201 { U2, U3, U0, U1, D0((HalfDead, Void)), Live(Result<Never, Option<char>>), D1((Void, ())) }
suite!(201, s201, E201, [E201::U2, E201::U3, E201::U0, E201::U1, E201::Live(Err(Some('z'))), E201::Live(Err(None))], |e| match e { E201::U2 => 1003, E201::U3 => 1004, E201::U0 => 1001, E201::U1 => 1002, E201::D0(..) => 9001, E201::Live(w) => { let p: Option<char> = (match w { Err(x) => x, Ok(_) => loop {} }); match p { Some(c) => c as u32, None => 48 } }, E201::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E202 { U4, U2, U5, Live(SL3<Sg3>), U0, U3, D0(Never), D2([[Void; 1]; 1]), D1((Void,)), U1 }
suite!(202, s202, E202, [E202::U4, E202::U2, E202::U5, E202::Live(SL3::Live(Sg3::A)), E202::Live(SL3::Live(Sg3::B)), E202::U0, E202::U3, E202::U1], |e| match e { E202::U4 => 1005, E202::U2 => 1003, E202::U5 => 1006, E202::Live(w) => { let p: Sg3 = (match w { SL3::Live(x) => x, _ => loop {} }); p as i32 as u32 }, E202::U0 => 1001, E202::U3 => 1004, E202::D0(..) => 9001, E202::D2(..) => 9003, E202::D1(..) => 9002, E202::U1 => 1002 });
#[derive(Clone, Copy)] enum E203 { D2([u32; 0], Void), D1(Result<(), Void>), Live(SL<Wrap<SL3<W64>>>), U0, U1, D0([[Void; 1]; 1]) }
suite!(203, s203, E203, [E203::D1(Ok(())), E203::Live(SL::Live(Wrap(SL3::Live(W64::A)))), E203::Live(SL::Live(Wrap(SL3::Live(W64::B)))), E203::U0, E203::U1], |e| match e { E203::D2(..) => 9003, E203::D1(..) => 2002, E203::Live(w) => { let p: W64 = (match w { SL::Live(Wrap(SL3::Live(x))) => x, _ => loop {} }); (p as u64 % 1_000_003) as u32 }, E203::U0 => 1001, E203::U1 => 1002, E203::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E204 { D0(Aligned), Live((Three,)) }
suite!(204, s204, E204, [E204::Live((Three::A,)), E204::Live((Three::B,)), E204::Live((Three::C,))], |e| match e { E204::D0(..) => 9001, E204::Live(w) => { let p: Three = w.0; p as u32 + 20 } });
#[derive(Clone, Copy)] enum E205 { D1((Void,)), Live(Hi3), D0(MaybeUninit<Void>), D2([u32; 0]) }
suite!(205, s205, E205, [E205::Live(Hi3::A), E205::Live(Hi3::B), E205::Live(Hi3::C), E205::D2([])], |e| match e { E205::D1(..) => 9002, E205::Live(w) => { let p: Hi3 = w; p as u32 }, E205::D0(..) => 9001, E205::D2(..) => 2003 });
#[derive(Clone, Copy)] enum E206 { U1, D0(TrDead), Live(Sg2), U0 }
suite!(206, s206, E206, [E206::U1, E206::Live(Sg2::A), E206::Live(Sg2::B), E206::U0], |e| match e { E206::U1 => 1002, E206::D0(..) => 9001, E206::Live(w) => { let p: Sg2 = w; p as i32 as u32 }, E206::U0 => 1001 });
#[derive(Clone, Copy)] enum E207 { D1([(Void,); 0]), Live(Result<Hi2, Void>), D0(Void), U1, U0 }
suite!(207, s207, E207, [E207::D1([]), E207::Live(Ok(Hi2::A)), E207::Live(Ok(Hi2::B)), E207::U1, E207::U0], |e| match e { E207::D1(..) => 2002, E207::Live(w) => { let p: Hi2 = (match w { Ok(x) => x, Err(_) => loop {} }); p as u32 }, E207::D0(..) => 9001, E207::U1 => 1002, E207::U0 => 1001 });
#[derive(Clone, Copy)] enum E208 { Live(SL<Wrap<SL3<Two>>>), D1([(Void,); 0]), D0((u32, Void)) }
suite!(208, s208, E208, [E208::Live(SL::Live(Wrap(SL3::Live(Two::A)))), E208::Live(SL::Live(Wrap(SL3::Live(Two::B)))), E208::D1([])], |e| match e { E208::Live(w) => { let p: Two = (match w { SL::Live(Wrap(SL3::Live(x))) => x, _ => loop {} }); p as u32 + 30 }, E208::D1(..) => 2002, E208::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E209 { U0, D0(Void, [u64; 0]), D1(MaybeUninit<Void>), U1, Live(Wrap<Tr<((), Option<bool>)>>) }
suite!(209, s209, E209, [E209::U0, E209::U1, E209::Live(Wrap(Tr(((), Some(true))))), E209::Live(Wrap(Tr(((), Some(false))))), E209::Live(Wrap(Tr(((), None))))], |e| match e { E209::U0 => 1001, E209::D0(..) => 9001, E209::D1(..) => 9002, E209::U1 => 1002, E209::Live(w) => { let p: Option<bool> = w.0.0.1; match p { Some(b) => 40 + b as u32, None => 49 } } });
#[derive(Clone, Copy)] enum E210 { D0(PkDead), U0, Live((S64, PhantomData<Void>, [Void; 0])) }
suite!(210, s210, E210, [E210::U0, E210::Live((S64::A, PhantomData, [])), E210::Live((S64::B, PhantomData, []))], |e| match e { E210::D0(..) => 9001, E210::U0 => 1001, E210::Live(w) => { let p: S64 = w.0; (p as i64 as u64 % 1_000_003) as u32 } });
#[derive(Clone, Copy)] enum E211 { Live((Option<Hi2>,)) }
suite!(211, s211, E211, [E211::Live((Some(Hi2::A),)), E211::Live((Some(Hi2::B),)), E211::Live((None,))], |e| match e { E211::Live(w) => { let p: Option<Hi2> = w.0; match p { Some(h) => h as u32, None => 47 } } });
#[derive(Clone, Copy)] enum E212 { Live(((), Hi2)), D1(PhantomData<Void>), D2(HalfDead2), D0(HalfDead), U0 }
suite!(212, s212, E212, [E212::Live(((), Hi2::A)), E212::Live(((), Hi2::B)), E212::D1(PhantomData), E212::D2(HalfDead2::A), E212::D0(HalfDead::B), E212::U0], |e| match e { E212::Live(w) => { let p: Hi2 = w.1; p as u32 }, E212::D1(..) => 2002, E212::D2(..) => 2003, E212::D0(..) => 2001, E212::U0 => 1001 });
#[derive(Clone, Copy)] enum E213 { Live(((), Small)), U1, U0, D0(Wrap<Void>), D1([(Void,); 3]) }
suite!(213, s213, E213, [E213::Live(((), Small::X)), E213::Live(((), Small::Y)), E213::U1, E213::U0], |e| match e { E213::Live(w) => { let p: Small = w.1; p as u32 }, E213::U1 => 1002, E213::U0 => 1001, E213::D0(..) => 9001, E213::D1(..) => 9002 });
#[derive(Clone, Copy)] enum E214 { Live(Result<Never, NonZeroU8>), U0 }
suite!(214, s214, E214, [E214::Live(Err(NonZeroU8::new(1).unwrap())), E214::Live(Err(NonZeroU8::new(255).unwrap())), E214::U0], |e| match e { E214::Live(w) => { let p: NonZeroU8 = (match w { Err(x) => x, Ok(_) => loop {} }); p.get() as u32 }, E214::U0 => 1001 });
#[derive(Clone, Copy)] enum E215 { Live(Result<Sg, Void>), D1([(Void,); 0]), D0(CDead), D2(&'static Void, Void) }
suite!(215, s215, E215, [E215::Live(Ok(Sg::A)), E215::Live(Ok(Sg::B)), E215::D1([])], |e| match e { E215::Live(w) => { let p: Sg = (match w { Ok(x) => x, Err(_) => loop {} }); p as i32 as u32 }, E215::D1(..) => 2002, E215::D0(..) => 9001, E215::D2(..) => 9003 });
#[derive(Clone, Copy)] enum E216 { U3, Live(ManuallyDrop<char>), D1(Void), U0, U2, D0(u8, Void), U1 }
suite!(216, s216, E216, [E216::U3, E216::Live(ManuallyDrop::new('a')), E216::Live(ManuallyDrop::new('\u{10FFFF}')), E216::Live(ManuallyDrop::new('\0')), E216::U0, E216::U2, E216::U1], |e| match e { E216::U3 => 1004, E216::Live(w) => { let p: char = ManuallyDrop::into_inner(w); p as u32 }, E216::D1(..) => 9002, E216::U0 => 1001, E216::U2 => 1003, E216::D0(..) => 9001, E216::U1 => 1002 });
#[derive(Clone, Copy)] enum E217 { D0(Wrap<HalfDead>, ()), Live(ZPad<fn() -> u32>) }
suite!(217, s217, E217, [E217::D0(Wrap(HalfDead::B), ()), E217::Live(ZPad([], seven, ())), E217::Live(ZPad([], eight, ()))], |e| match e { E217::D0(..) => 2001, E217::Live(w) => { let p: fn() -> u32 = w.1; p() } });
#[derive(Clone, Copy)] enum E218 { Live(((), Sg2)), D2([(Void,); 3]), D0(CDead), U1, U2, D1(Void), U0 }
suite!(218, s218, E218, [E218::Live(((), Sg2::A)), E218::Live(((), Sg2::B)), E218::U1, E218::U2, E218::U0], |e| match e { E218::Live(w) => { let p: Sg2 = w.1; p as i32 as u32 }, E218::D2(..) => 9003, E218::D0(..) => 9001, E218::U1 => 1002, E218::U2 => 1003, E218::D1(..) => 9002, E218::U0 => 1001 });
#[derive(Clone, Copy)] enum E219 { U0, U1, Live(CW<Small>) }
suite!(219, s219, E219, [E219::U0, E219::U1, E219::Live(CW(Small::X)), E219::Live(CW(Small::Y))], |e| match e { E219::U0 => 1001, E219::U1 => 1002, E219::Live(w) => { let p: Small = w.0; p as u32 } });

#[unsafe(no_mangle)]
pub fn rust_entry() {
    s201(); s202(); s203(); s204(); s205(); s206(); s207(); s208(); s209(); s210(); s211(); s212(); s213(); s214(); s215(); s216(); s217(); s218(); s219();
}
