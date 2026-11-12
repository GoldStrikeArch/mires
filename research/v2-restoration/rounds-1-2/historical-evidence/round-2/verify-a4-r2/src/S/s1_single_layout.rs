#![no_std]
#![no_main]
#![allow(unused, unreachable_patterns, dead_code)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::convert::Infallible;
use core::hint::black_box;
use core::marker::PhantomData;
use core::mem::discriminant;
use core::ops::ControlFlow;

trait HasOut { type Out; }
impl<T> HasOut for fn() -> T { type Out = T; }
type Never = <fn() -> ! as HasOut>::Out;
#[derive(Clone, Copy)] enum Void {}
#[derive(Clone, Copy)] struct Wrap<T>(T);

// zero-sized enums laid out as one variant
#[derive(Clone, Copy)] enum HD1 { A(Void), B }
#[derive(Clone, Copy)] enum HD2 { A, B(Never) }
#[derive(Clone, Copy)] enum HD3 { A((Void,)), B }
#[derive(Clone, Copy)] enum HD4 { A(Wrap<Void>), B(Void), C, D([Void; 2]) }
#[derive(Clone, Copy)] enum HD5 { A(Void), B(()) }
#[derive(Clone, Copy)] enum HD6 { A(Infallible), B(PhantomData<u8>, ()) }
#[derive(Clone, Copy)] enum HD7 { A([Void; 0]), B(Void) }
#[derive(Clone, Copy)] enum HD9 { A(Wrap<Wrap<(Void, ())>>), B([[Void; 1]; 1]), C(HD1) }
// sized ones
#[derive(Clone, Copy)] enum SL<T> { Never((), Infallible), Live(T) }
#[derive(Clone, Copy)] enum SL3<T> { D0(Void), D1(Never), Live(T), D3((Void, ())) }
#[derive(Clone, Copy)] enum S5 { A(Wrap<Void>), B { x: u32, y: u8 } }
#[derive(Clone, Copy)] enum S6 { A(Void), B(&'static str) }
#[derive(Clone, Copy)] enum S7 { A([Void; 1]), B(Option<u32>) }
#[derive(Clone, Copy)] enum S8 { A((Void, ())), B(u64, u64) }

macro_rules! chk {
    ($T:ty, $k:expr, |$e:ident| $body:expr) => {{
        #[inline(never)] fn cls($e: $T) -> u32 { $body }
        #[inline(never)] fn cls_ref(r: &$T) -> u32 { let $e = *r; $body }
        #[inline(never)] fn mk() -> $T { $k }
        const K: $T = $k;
        static S: $T = $k;
        static TAB: [$T; 2] = [$k, $k];
        { let $e = K; print_u32($body); }
        print_u32(cls(K));
        print_u32(cls(black_box(K)));
        print_u32(cls_ref(&S));
        print_u32(cls(TAB[black_box(1)]));
        print_u32(cls(mk()));
        { let $e = black_box(mk()); print_u32($body); }
        print_bool(discriminant(&K) == discriminant(&mk()));
        print_bool(discriminant(black_box(&S)) == discriminant(&TAB[black_box(0)]));
        const O: Option<$T> = Some($k);
        const N: Option<$T> = None;
        print_u32(match black_box(O) { Some($e) => $body, None => 55555 });
        print_u32(match black_box(N) { Some($e) => $body, None => 55555 });
        print_u32(match O { Some($e) => $body, None => 55555 });
        let o = Some(mk());
        print_u32(match black_box(o) { Some($e) => $body, None => 55555 });
        const OO: Option<Option<$T>> = Some(Some($k));
        print_u32(match black_box(OO) { Some(Some($e)) => $body, Some(None) => 55556, None => 55557 });
        const ON: Option<Option<$T>> = Some(None);
        print_u32(match black_box(ON) { Some(Some($e)) => $body, Some(None) => 55556, None => 55557 });
        let third: $T = $k;
        let pair = (mk(), 7u32, third);
        { let $e = black_box(pair).2; print_u32($body + pair.1); }
    }};
}

#[inline(never)]
fn generic_ok<T: Copy>(r: Result<T, Void>) -> bool { r.is_ok() }
#[inline(never)]
fn generic_err<T: Copy>(r: Result<(Infallible, ()), T>) -> bool { matches!(r, Err(_)) }

#[unsafe(no_mangle)]
pub fn rust_entry() {
    chk!(HD1, HD1::B, |e| match e { HD1::A(_) => 99, HD1::B => 1 });
    chk!(HD2, HD2::A, |e| match e { HD2::A => 2, HD2::B(_) => 99 });
    chk!(HD3, HD3::B, |e| match e { HD3::A(_) => 99, HD3::B => 3 });
    chk!(HD4, HD4::C, |e| match e { HD4::C => 4, _ => 99 });
    chk!(HD5, HD5::B(()), |e| match e { HD5::A(_) => 99, HD5::B(()) => 5 });
    chk!(HD6, HD6::B(PhantomData, ()), |e| match e { HD6::A(_) => 99, HD6::B(..) => 6 });
    chk!(HD7, HD7::A([]), |e| match e { HD7::A(_) => 7, HD7::B(_) => 99 });
    chk!(HD9, HD9::C(HD1::B), |e| match e { HD9::C(HD1::B) => 9, HD9::C(_) => 98, _ => 99 });
    chk!(SL<char>, SL::Live('q'), |e| match e { SL::Live(c) => c as u32, SL::Never(..) => 99 });
    chk!(SL<u64>, SL::Live(0x1_0000_0005), |e| match e { SL::Live(c) => (c % 1000) as u32, SL::Never(..) => 99 });
    chk!(SL3<u32>, SL3::Live(33), |e| match e { SL3::Live(c) => c, _ => 99 });
    chk!(SL3<HD1>, SL3::Live(HD1::B), |e| match e { SL3::Live(HD1::B) => 34, _ => 99 });
    chk!(SL<SL3<bool>>, SL::Live(SL3::Live(true)), |e| match e { SL::Live(SL3::Live(b)) => 40 + b as u32, _ => 99 });
    chk!(S5, S5::B { x: 50, y: 5 }, |e| match e { S5::B { x, y } => x + y as u32, S5::A(_) => 99 });
    chk!(S6, S6::B("six"), |e| match e { S6::B(s) => s.len() as u32 + 60, S6::A(_) => 99 });
    chk!(S7, S7::B(Some(70)), |e| match e { S7::B(Some(n)) => n, S7::B(None) => 71, S7::A(_) => 99 });
    chk!(S7, S7::B(None), |e| match e { S7::B(Some(n)) => n, S7::B(None) => 71, S7::A(_) => 99 });
    chk!(S8, S8::B(80, 1), |e| match e { S8::B(a, b) => (a + b) as u32, S8::A(_) => 99 });
    chk!(Result<u32, Void>, Ok(90), |e| match e { Ok(n) => n, Err(_) => 99 });
    chk!(Result<Void, u32>, Err(91), |e| match e { Ok(_) => 99, Err(n) => n });
    chk!(Result<(Infallible, ()), char>, Err('r'), |e| match e { Ok(_) => 99, Err(c) => c as u32 });
    chk!(Result<Wrap<Never>, (u32, u32)>, Err((92, 1)), |e| match e { Ok(_) => 99, Err((a, b)) => a + b });
    chk!(Result<(), [Void; 1]>, Ok(()), |e| match e { Ok(()) => 93, Err(_) => 99 });
    chk!(Result<[Void; 1], ()>, Err(()), |e| match e { Ok(_) => 99, Err(()) => 94 });
    chk!(ControlFlow<Infallible, u32>, ControlFlow::Continue(95), |e| match e { ControlFlow::Continue(n) => n, ControlFlow::Break(_) => 99 });
    chk!(ControlFlow<(Void, ()), u32>, ControlFlow::Continue(96), |e| match e { ControlFlow::Continue(n) => n, ControlFlow::Break(_) => 99 });
    chk!(ControlFlow<u32, Wrap<Void>>, ControlFlow::Break(97), |e| match e { ControlFlow::Continue(_) => 99, ControlFlow::Break(n) => n });
    print_bool(generic_ok(black_box(Ok::<u32, Void>(1))));
    print_bool(generic_ok(Ok::<HD1, Void>(HD1::B)));
    print_bool(generic_err(black_box(Err::<(Infallible, ()), char>('x'))));
    print_bool(generic_err(Err::<(Infallible, ()), ()>(())));
    let r: Result<u32, (Void, ())> = Ok(black_box(5));
    print_bool(r.is_ok()); print_u32(r.map_or(0, |n| n + 1)); print_u32(match r { Ok(n) => n, Err(_) => 99 });
    let c: ControlFlow<Wrap<Void>, u32> = ControlFlow::Continue(black_box(6));
    print_bool(c.is_continue()); print_bool(matches!(c, ControlFlow::Continue(6)));
}
