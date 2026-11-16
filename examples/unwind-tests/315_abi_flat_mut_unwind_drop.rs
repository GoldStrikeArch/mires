//! Negative control 1 for Slice R3-A: host-throw inside `&mut Enum` / `&mut Scalar` callee
//! must decline copy-back (`ExceptionalCopyBack`) so caller `Drop` observers see pre-throw writes.

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
fn step_and_throw(state: &mut Phase, acc: &mut i32, trigger: bool) {
    match *state {
        Phase::Alpha => *state = Phase::Beta { x: 10, y: 20 },
        Phase::Beta { .. } | Phase::Ready { .. } => {}
    }
    *state = Phase::Ready { total: 77 };
    *acc = 99;
    if trigger {
        unsafe { host_note(-1) };
    }
}

#[unsafe(no_mangle)]
pub fn run_unwind_observer(trigger: bool) -> i32 {
    let mut obs = Observer {
        state: Phase::Alpha,
        acc: 1,
    };
    step_and_throw(&mut obs.state, &mut obs.acc, trigger);
    obs.acc
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}
