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
#[derive(Clone, Copy)] enum E240 { U5, U0, Live((NonZeroI8,)), U4, U3, U1, U2 }
suite!(240, s240, E240, [E240::U5, E240::U0, E240::Live((NonZeroI8::new(-1).unwrap(),)), E240::Live((NonZeroI8::new(127).unwrap(),)), E240::U4, E240::U3, E240::U1, E240::U2], [E240::Live((black_box(NonZeroI8::new(-1).unwrap()),)), E240::Live((black_box(NonZeroI8::new(127).unwrap()),))], |e| match e { E240::U5 => 1006, E240::U0 => 1001, E240::Live(w) => { let p: NonZeroI8 = w.0; p.get() as i32 as u32 }, E240::U4 => 1005, E240::U3 => 1004, E240::U1 => 1002, E240::U2 => 1003 });
#[derive(Clone, Copy)] enum E241 { U0, D0((Never, Never)), D1(&'static Void, Void), Live(SL3<Sg3>) }
suite!(241, s241, E241, [E241::U0, E241::Live(SL3::Live(Sg3::A)), E241::Live(SL3::Live(Sg3::B))], [E241::Live(SL3::Live(black_box(Sg3::A))), E241::Live(SL3::Live(black_box(Sg3::B)))], |e| match e { E241::U0 => 1001, E241::D0(..) => 9001, E241::D1(..) => 9002, E241::Live(w) => { let p: Sg3 = (match w { SL3::Live(x) => x, _ => loop {} }); p as i32 as u32 } });
#[derive(Clone, Copy)] enum E242 { U0, Live([Small; 1]), U4, U3, U1, U5, U2, D0(HalfDead) }
suite!(242, s242, E242, [E242::U0, E242::Live([Small::X]), E242::Live([Small::Y]), E242::U4, E242::U3, E242::U1, E242::U5, E242::U2, E242::D0(HalfDead::B)], [E242::Live([black_box(Small::X)]), E242::Live([black_box(Small::Y)])], |e| match e { E242::U0 => 1001, E242::Live(w) => { let p: Small = w[0]; p as u32 }, E242::U4 => 1005, E242::U3 => 1004, E242::U1 => 1002, E242::U5 => 1006, E242::U2 => 1003, E242::D0(..) => 2001 });
#[derive(Clone, Copy)] enum E243 { U3, U0, U1, D0(u8, Void), D1(DeepDead), Live(Result<Hi2, Void>), U2 }
suite!(243, s243, E243, [E243::U3, E243::U0, E243::U1, E243::Live(Ok(Hi2::A)), E243::Live(Ok(Hi2::B)), E243::U2], [E243::Live(Ok(black_box(Hi2::A))), E243::Live(Ok(black_box(Hi2::B)))], |e| match e { E243::U3 => 1004, E243::U0 => 1001, E243::U1 => 1002, E243::D0(..) => 9001, E243::D1(..) => 9002, E243::Live(w) => { let p: Hi2 = (match w { Ok(x) => x, Err(_) => loop {} }); p as u32 }, E243::U2 => 1003 });
#[derive(Clone, Copy)] enum E244 { Live(Wrap<W32>), U0, D0(Wrap<Wrap<(Void, ())>>) }
suite!(244, s244, E244, [E244::Live(Wrap(W32::A)), E244::Live(Wrap(W32::B)), E244::U0], [E244::Live(Wrap(black_box(W32::A))), E244::Live(Wrap(black_box(W32::B)))], |e| match e { E244::Live(w) => { let p: W32 = w.0; p as u32 }, E244::U0 => 1001, E244::D0(..) => 9001 });
#[derive(Clone, Copy)] enum E245 { U4, U2, U3, Live((NonZeroU8, PhantomData<Void>, [Void; 0])), U1, U0, U5 }
suite!(245, s245, E245, [E245::U4, E245::U2, E245::U3, E245::Live((NonZeroU8::new(1).unwrap(), PhantomData, [])), E245::Live((NonZeroU8::new(255).unwrap(), PhantomData, [])), E245::U1, E245::U0, E245::U5], [E245::Live((black_box(NonZeroU8::new(1).unwrap()), PhantomData, [])), E245::Live((black_box(NonZeroU8::new(255).unwrap()), PhantomData, []))], |e| match e { E245::U4 => 1005, E245::U2 => 1003, E245::U3 => 1004, E245::Live(w) => { let p: NonZeroU8 = w.0; p.get() as u32 }, E245::U1 => 1002, E245::U0 => 1001, E245::U5 => 1006 });
#[derive(Clone, Copy)] enum E246 { Live(CW<Sg>), D1(Void), D0((u32, Void)), D2((HalfDead, Void)) }
suite!(246, s246, E246, [E246::Live(CW(Sg::A)), E246::Live(CW(Sg::B))], [E246::Live(CW(black_box(Sg::A))), E246::Live(CW(black_box(Sg::B)))], |e| match e { E246::Live(w) => { let p: Sg = w.0; p as i32 as u32 }, E246::D1(..) => 9002, E246::D0(..) => 9001, E246::D2(..) => 9003 });
#[derive(Clone, Copy)] enum E247 { Live(ManuallyDrop<Option<char>>) }
suite!(247, s247, E247, [E247::Live(ManuallyDrop::new(Some('z'))), E247::Live(ManuallyDrop::new(None))], [E247::Live(ManuallyDrop::new(black_box(Some('z')))), E247::Live(ManuallyDrop::new(black_box(None)))], |e| match e { E247::Live(w) => { let p: Option<char> = ManuallyDrop::into_inner(w); match p { Some(c) => c as u32, None => 48 } } });
#[derive(Clone, Copy)] enum E248 { D0(CDead), Live(Option<char>) }
suite!(248, s248, E248, [E248::Live(Some('z')), E248::Live(None)], [E248::Live(black_box(Some('z'))), E248::Live(black_box(None))], |e| match e { E248::D0(..) => 9001, E248::Live(w) => { let p: Option<char> = w; match p { Some(c) => c as u32, None => 48 } } });
#[derive(Clone, Copy)] enum E249 { D0([u16; 0], Void), Live((NonZeroU8,)) }
suite!(249, s249, E249, [E249::Live((NonZeroU8::new(1).unwrap(),)), E249::Live((NonZeroU8::new(255).unwrap(),))], [E249::Live((black_box(NonZeroU8::new(1).unwrap()),)), E249::Live((black_box(NonZeroU8::new(255).unwrap()),))], |e| match e { E249::D0(..) => 9001, E249::Live(w) => { let p: NonZeroU8 = w.0; p.get() as u32 } });
#[derive(Clone, Copy)] enum E250 { U0, Live(Wrap<bool>), D2(Aligned), U5, U1, U3, D1(HalfDead), D0([u32; 0]), U4, U2 }
suite!(250, s250, E250, [E250::U0, E250::Live(Wrap(true)), E250::Live(Wrap(false)), E250::U5, E250::U1, E250::U3, E250::D1(HalfDead::B), E250::D0([]), E250::U4, E250::U2], [E250::Live(Wrap(black_box(true))), E250::Live(Wrap(black_box(false)))], |e| match e { E250::U0 => 1001, E250::Live(w) => { let p: bool = w.0; 10 + p as u32 }, E250::D2(..) => 9003, E250::U5 => 1006, E250::U1 => 1002, E250::U3 => 1004, E250::D1(..) => 2002, E250::D0(..) => 2001, E250::U4 => 1005, E250::U2 => 1003 });
#[derive(Clone, Copy)] enum E251 { Live(Pk<char>), U0, D0(Wrap<HalfDead>, ()), U2, U1 }
suite!(251, s251, E251, [E251::Live(Pk('a')), E251::Live(Pk('\u{10FFFF}')), E251::Live(Pk('\0')), E251::U0, E251::D0(Wrap(HalfDead::B), ()), E251::U2, E251::U1], [E251::Live(Pk(black_box('a'))), E251::Live(Pk(black_box('\u{10FFFF}'))), E251::Live(Pk(black_box('\0')))], |e| match e { E251::Live(w) => { let p: char = { w.0 }; p as u32 }, E251::U0 => 1001, E251::D0(..) => 2001, E251::U2 => 1003, E251::U1 => 1002 });
#[derive(Clone, Copy)] enum E252 { D2([u16; 0], Void), D0(Result<Void, Never>), D1(NamedDead), U0, Live(Tr<W16>), U1 }
suite!(252, s252, E252, [E252::U0, E252::Live(Tr(W16::A)), E252::Live(Tr(W16::B)), E252::U1], [E252::Live(Tr(black_box(W16::A))), E252::Live(Tr(black_box(W16::B)))], |e| match e { E252::D2(..) => 9003, E252::D0(..) => 9001, E252::D1(..) => 9002, E252::U0 => 1001, E252::Live(w) => { let p: W16 = w.0; p as u32 }, E252::U1 => 1002 });
#[derive(Clone, Copy)] enum E253 { D0(PkDead), Live(ManuallyDrop<S16>), U0, U1 }
suite!(253, s253, E253, [E253::Live(ManuallyDrop::new(S16::A)), E253::Live(ManuallyDrop::new(S16::B)), E253::U0, E253::U1], [E253::Live(ManuallyDrop::new(black_box(S16::A))), E253::Live(ManuallyDrop::new(black_box(S16::B)))], |e| match e { E253::D0(..) => 9001, E253::Live(w) => { let p: S16 = ManuallyDrop::into_inner(w); p as i32 as u32 }, E253::U0 => 1001, E253::U1 => 1002 });
#[derive(Clone, Copy)] enum E254 { Live(Hi3), U0, D1([(Void,); 3]), D0(PhantomData<Void>) }
suite!(254, s254, E254, [E254::Live(Hi3::A), E254::Live(Hi3::B), E254::Live(Hi3::C), E254::U0, E254::D0(PhantomData)], [E254::Live(black_box(Hi3::A)), E254::Live(black_box(Hi3::B)), E254::Live(black_box(Hi3::C))], |e| match e { E254::Live(w) => { let p: Hi3 = w; p as u32 }, E254::U0 => 1001, E254::D1(..) => 9002, E254::D0(..) => 2001 });
#[derive(Clone, Copy)] enum E255 { D2([u16; 0], Void), D0((Void,)), Live(Pk<Sg2>), D1([u32; 0]) }
suite!(255, s255, E255, [E255::Live(Pk(Sg2::A)), E255::Live(Pk(Sg2::B)), E255::D1([])], [E255::Live(Pk(black_box(Sg2::A))), E255::Live(Pk(black_box(Sg2::B)))], |e| match e { E255::D2(..) => 9003, E255::D0(..) => 9001, E255::Live(w) => { let p: Sg2 = { w.0 }; p as i32 as u32 }, E255::D1(..) => 2002 });
#[derive(Clone, Copy)] enum E256 { U1, Live(SL3<S64>), U4, U0, U2, U5, D1([Void; 2]), D0([Void; 0]), U3, D2([u16; 0], Void) }
suite!(256, s256, E256, [E256::U1, E256::Live(SL3::Live(S64::A)), E256::Live(SL3::Live(S64::B)), E256::U4, E256::U0, E256::U2, E256::U5, E256::D0([]), E256::U3], [E256::Live(SL3::Live(black_box(S64::A))), E256::Live(SL3::Live(black_box(S64::B)))], |e| match e { E256::U1 => 1002, E256::Live(w) => { let p: S64 = (match w { SL3::Live(x) => x, _ => loop {} }); (p as i64 as u64 % 1_000_003) as u32 }, E256::U4 => 1005, E256::U0 => 1001, E256::U2 => 1003, E256::U5 => 1006, E256::D1(..) => 9002, E256::D0(..) => 2001, E256::U3 => 1004, E256::D2(..) => 9003 });
#[derive(Clone, Copy)] enum E257 { U0, Live(CW<Sg>), D0(HalfDead2) }
suite!(257, s257, E257, [E257::U0, E257::Live(CW(Sg::A)), E257::Live(CW(Sg::B)), E257::D0(HalfDead2::A)], [E257::Live(CW(black_box(Sg::A))), E257::Live(CW(black_box(Sg::B)))], |e| match e { E257::U0 => 1001, E257::Live(w) => { let p: Sg = w.0; p as i32 as u32 }, E257::D0(..) => 2001 });
#[derive(Clone, Copy)] enum E258 { U0, U1, Live(Wrap<Tr<((), Option<Hi2>)>>) }
suite!(258, s258, E258, [E258::U0, E258::U1, E258::Live(Wrap(Tr(((), Some(Hi2::A))))), E258::Live(Wrap(Tr(((), Some(Hi2::B))))), E258::Live(Wrap(Tr(((), None))))], [E258::Live(Wrap(Tr(((), black_box(Some(Hi2::A)))))), E258::Live(Wrap(Tr(((), black_box(Some(Hi2::B)))))), E258::Live(Wrap(Tr(((), black_box(None)))))], |e| match e { E258::U0 => 1001, E258::U1 => 1002, E258::Live(w) => { let p: Option<Hi2> = w.0.0.1; match p { Some(h) => h as u32, None => 47 } } });
#[derive(Clone, Copy)] enum E259 { D0([u16; 0], Void), U3, U2, Live(Named<S64>), D2(ManuallyDrop<Void>), D1([Void; 0]), U1, U0 }
suite!(259, s259, E259, [E259::U3, E259::U2, E259::Live(Named { z: (), v: S64::A, p: PhantomData }), E259::Live(Named { z: (), v: S64::B, p: PhantomData }), E259::D1([]), E259::U1, E259::U0], [E259::Live(Named { z: (), v: black_box(S64::A), p: PhantomData }), E259::Live(Named { z: (), v: black_box(S64::B), p: PhantomData })], |e| match e { E259::D0(..) => 9001, E259::U3 => 1004, E259::U2 => 1003, E259::Live(w) => { let p: S64 = w.v; (p as i64 as u64 % 1_000_003) as u32 }, E259::D2(..) => 9003, E259::D1(..) => 2002, E259::U1 => 1002, E259::U0 => 1001 });

#[unsafe(no_mangle)]
pub fn rust_entry() {
    s240(); s241(); s242(); s243(); s244(); s245(); s246(); s247(); s248(); s249(); s250(); s251(); s252(); s253(); s254(); s255(); s256(); s257(); s258(); s259();
}
