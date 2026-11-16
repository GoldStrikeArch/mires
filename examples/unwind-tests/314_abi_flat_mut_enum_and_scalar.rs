//! Positive test for Slice R3-A: private direct-call `&mut Enum` + `&mut i32` flattening.

#![no_std]
#![no_main]

enum State {
    Idle,
    Loading { progress: i32, retries: i32 },
    Ready { value: i32, flags: i32, checksum: i32 },
}

#[inline(never)]
fn step_state(state: &mut State, i: i32, acc: &mut i32) {
    match *state {
        State::Idle => {
            *state = State::Loading {
                progress: i,
                retries: 1,
            };
        }
        State::Loading { progress, retries } => {
            if (i & 3) == 3 {
                *state = State::Ready {
                    value: progress.wrapping_add(i),
                    flags: retries,
                    checksum: i.wrapping_mul(31),
                };
            } else {
                *state = State::Loading {
                    progress: progress.wrapping_add(i),
                    retries: retries.wrapping_add(1),
                };
            }
        }
        State::Ready {
            value,
            flags,
            checksum,
        } => {
            *acc = acc
                .wrapping_add(value)
                .wrapping_add(flags)
                .wrapping_add(checksum);
            *state = State::Idle;
        }
    }
}

#[unsafe(no_mangle)]
pub fn bench_enum_state(n: i32) -> i32 {
    let mut state = State::Idle;
    let mut acc = 0;
    for i in 0..n {
        step_state(&mut state, i, &mut acc);
    }
    acc
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}
