// F4: coroutine state captured by value.
use core::future::Future;
use core::pin::{pin, Pin};
use core::task::{Context, Poll, Waker};
fn block<F: Future>(f: F) -> F::Output {
    let mut f = pin!(f);
    let mut cx = Context::from_waker(Waker::noop());
    loop { if let Poll::Ready(v) = f.as_mut().poll(&mut cx) { return v; } }
}
struct Yield(bool);
impl Future for Yield {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> { if self.0 { Poll::Ready(()) } else { self.0 = true; Poll::Pending } }
}
async fn bump(mut p: P) -> u32 { p.x += 1; Yield(false).await; p.x += 1; p.x }
async fn bump_w(mut w: W) -> u32 { w.0 += 1; Yield(false).await; w.0 += 1; w.0 }
async fn bump_arr(mut a: [u32; 3]) -> u32 { a[1] += 1; Yield(false).await; a[bb(1)] += 1; a[1] }
async fn bump_ref(p: &P) -> u32 { let mut q = *p; q.x += 1; Yield(false).await; q.x += 1; q.x }

#[inline(never)] fn y_async_move(s: &[P]) -> u32 { let p = s[bb(1)]; let r = block(async move { let mut q = p; q.x = 99; Yield(false).await; q.y = 98; q.x + q.y }); r * 1000 + p.x + s[1].y }
#[inline(never)] fn y_async_capture_mut(s: &[P]) -> u32 { let mut p = s[bb(1)]; let r = block(async move { p.x += 1; Yield(false).await; p.x += 1; p.x }); r * 1000 + p.x + s[1].x }
#[inline(never)] fn y_async_arr(a: &[[u32; 3]]) -> u32 { let mut c = a[bb(1)]; let r = block(async move { c[0] = 99; Yield(false).await; c[1] = 98; c[0] + c[1] }); r * 1000 + c[0] + a[1][0] }
#[inline(never)] fn y_async_fn(s: &[P]) -> u32 { block(bump(s[bb(1)])) * 1000 + s[1].x }
#[inline(never)] fn y_async_fn_param(p: P) -> u32 { block(bump(p)) * 1000 + p.x }
#[inline(never)] fn y_async_fn_twice(p: P) -> u32 { let a = bump(p); let b = bump(p); (block(a) + block(b)) * 1000 + p.x }
#[inline(never)] fn y_async_w(w: W) -> u32 { block(bump_w(w)) * 1000 + w.0 }
#[inline(never)] fn y_async_w_slice(ws: &[W]) -> u32 { block(bump_w(ws[bb(1)])) * 1000 + ws[1].0 }
#[inline(never)] fn y_async_w_move(w: W) -> u32 { let mut v = w; let r = block(async move { v.0 += 1; Yield(false).await; v.0 += 1; v.0 }); r * 1000 + v.0 * 10 + w.0 }
#[inline(never)] fn y_async_arr_fn(a: &[[u32; 3]]) -> u32 { block(bump_arr(a[bb(1)])) * 1000 + a[1][1] }
#[inline(never)] fn y_async_ref(s: &[P]) -> u32 { block(bump_ref(&s[bb(1)])) * 1000 + s[1].x }
#[inline(never)] fn y_async_fresh(n: u32, m: u32) -> u32 { let r = block(async move { let mut q = P { x: n, y: m }; q.x += 1; Yield(false).await; q.y += 1; q.x * 10 + q.y }); r * 10000 + n * 100 + m }
#[inline(never)] fn y_async_fresh_arg(n: u32, m: u32) -> u32 { block(bump(P { x: n, y: m })) * 10000 + n * 100 + m }
#[inline(never)] fn y_async_nested(s: &[P]) -> u32 { let p = s[bb(1)]; let r = block(async move { let a = bump(p).await; let b = bump(p).await; let mut q = p; q.x += 10; Yield(false).await; a * 100 + b + q.x }); r * 1000 + p.x + s[1].x }
#[inline(never)] fn y_async_loop(s: &[P]) -> u32 { let r = block(async { let mut t = 0; for i in 0..s.len() { let mut q = s[i]; q.x += 1; Yield(false).await; q.y += q.x; t += q.y; } t }); r * 1000 + sum(s) }
#[inline(never)] fn y_async_across(s: &[P]) -> u32 { let r = block(async { let mut q = s[bb(1)]; let r = &mut q; r.x = 99; Yield(false).await; r.y = 98; q.x + q.y }); r * 1000 + s[1].x + s[1].y }
#[inline(never)] fn y_async_cell(n: u32, m: u32) -> u32 { let r = block(async move { let c = Cell::new(n); c.set(99); Yield(false).await; let d = Cell::new(m); d.set(d.get() + 1); c.get() * 100 + d.get() }); r * 10000 + n * 100 + m }

#[unsafe(no_mangle)]
pub fn rust_entry() {
    for _ in 0..2 {
        let ps = bb([P { x: 1, y: 2 }, P { x: 3, y: 4 }, P { x: 5, y: 6 }]);
        let s = &ps[..];
        let aa = bb([[1u32, 2, 3], [4, 5, 6]]);
        let ws = bb([W(10), W(11), W(12)]);
        let p78 = bb(P { x: 7, y: 8 });
        t!("y_async_move", y_async_move(s));
        t!("y_async_capture_mut", y_async_capture_mut(s));
        t!("y_async_arr", y_async_arr(&aa));
        t!("y_async_fn", y_async_fn(s));
        t!("y_async_fn_param", y_async_fn_param(p78));
        t!("y_async_fn_twice", y_async_fn_twice(p78));
        t!("y_async_w", y_async_w(bb(W(5))));
        t!("y_async_w_slice", y_async_w_slice(&ws));
        t!("y_async_w_move", y_async_w_move(bb(W(5))));
        t!("y_async_arr_fn", y_async_arr_fn(&aa));
        t!("y_async_ref", y_async_ref(s));
        t!("y_async_fresh", y_async_fresh(bb(3), bb(4)));
        t!("y_async_fresh_arg", y_async_fresh_arg(bb(3), bb(4)));
        t!("y_async_nested", y_async_nested(s));
        t!("y_async_loop", y_async_loop(s));
        t!("y_async_across", y_async_across(s));
        t!("y_async_cell", y_async_cell(bb(3), bb(4)));
        t!("after", sum(s) + suma(&aa[0]) + suma(&aa[1]) + ws[1].0 + p78.x);
    }
}
