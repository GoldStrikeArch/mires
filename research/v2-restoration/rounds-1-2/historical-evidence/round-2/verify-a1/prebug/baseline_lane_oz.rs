//! Baseline lane (default profile) under JSC_RUST_OPT_LEVEL=z JSC_RUST_MIR_INLINE=on only; right in
//! every JIR configuration and in the default profile without those settings. Main and the A1
//! commit emit identical JavaScript.
//! Native (-O0 and -O3): 304 / 99006 / 18007
#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::cell::Cell;
use core::future::Future;
use core::hint::black_box;
use core::pin::{pin, Pin};
use core::task::{Context, Poll, Waker};

#[derive(Clone, Copy)]
struct P {
    x: u32,
    y: u32,
}

struct C {
    a: Cell<u32>,
    b: u32,
}

// The implementer's shape: the store through the shared view writes the operand `n`.
#[inline(never)]
fn cell_set_dead(n: u32, m: u32) -> u32 {
    let q = C { a: Cell::new(n), b: m };
    q.a.set(99);
    n * 100 + m
}

// A LOST write: the cells view of a `Cell<[u32; 3]>` is a copy.
#[inline(never)]
fn array_of_cells(a: &[[u32; 3]]) -> u32 {
    let c = Cell::new(a[black_box(1)]);
    let cs: &[Cell<u32>; 3] = c.as_array_of_cells();
    cs[2].set(99);
    c.get()[2] * 1000 + a[1][2]
}

struct Yield(bool);
impl Future for Yield {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
        if self.0 {
            Poll::Ready(())
        } else {
            self.0 = true;
            Poll::Pending
        }
    }
}

fn block<F: Future>(f: F) -> F::Output {
    let mut f = pin!(f);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
    }
}

async fn bump(mut p: P) -> u32 {
    p.x += 1;
    Yield(false).await;
    p.x += 1;
    p.x
}

// Two futures built from one argument: the second panics "`async fn` resumed after completion".
#[inline(never)]
fn two_futures(p: P) -> u32 {
    let a = bump(p);
    let b = bump(p);
    (block(a) + block(b)) * 1000 + p.x
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(cell_set_dead(black_box(3), black_box(4)));
    print_u32(array_of_cells(&black_box([[1u32, 2, 3], [4, 5, 6]])));
    print_u32(two_futures(black_box(P { x: 7, y: 8 })));
}
