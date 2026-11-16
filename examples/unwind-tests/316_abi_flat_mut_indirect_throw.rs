//! Negative control 2 for Slice R3-A: unclosed indirect call inside `&mut Enum` / `&mut Scalar`
//! callee can throw and must mark `may_unwind = true`, declining copy-back with
//! `ExceptionalCopyBack`.

#![no_std]
#![no_main]

unsafe extern "C-unwind" {
    fn host_note(value: i32);
}

enum Phase {
    Alpha,
    Beta { x: i32, y: i32 },
    Ready { total: i32 },
}

struct Observer {
    state: Phase,
    acc: i32,
}

impl Drop for Observer {
    fn drop(&mut self) {
        unsafe {
            host_note(self.acc);
            let tag_or_total = match self.state {
                Phase::Alpha => 0,
                Phase::Beta { x, y } => x + y,
                Phase::Ready { total } => total,
            };
            host_note(tag_or_total);
        }
    }
}

#[inline(never)]
fn step_via_indirect(state: &mut Phase, acc: &mut i32, cb: unsafe extern "C-unwind" fn(i32)) {
    match *state {
        Phase::Alpha => *state = Phase::Beta { x: 10, y: 20 },
        Phase::Beta { .. } | Phase::Ready { .. } => {}
    }
    *state = Phase::Ready { total: 77 };
    *acc = 99;
    unsafe { cb(-1) };
}

#[unsafe(no_mangle)]
pub fn run_indirect_observer(use_throwing: bool) -> i32 {
    unsafe extern "C-unwind" fn noop(_: i32) {}
    let mut obs = Observer {
        state: Phase::Alpha,
        acc: 1,
    };
    let cb: unsafe extern "C-unwind" fn(i32) = if use_throwing { host_note } else { noop };
    step_via_indirect(&mut obs.state, &mut obs.acc, cb);
    obs.acc
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}
