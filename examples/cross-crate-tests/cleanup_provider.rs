#![no_std]
#![deny(unsafe_code)]

use mires_js::{await_promise, js_bindings, JsValue};

pub fn identity<T>(value: T) -> T {
    value
}

js_bindings! {
    #[js(call = "activePollNote")]
    fn note(value: f64);
}

struct Guard(u32);

impl Drop for Guard {
    fn drop(&mut self) {
        note(f64::from(self.0));
    }
}

struct Pair {
    _first: Guard,
    _second: Guard,
}

#[inline(never)]
fn make_guard(value: u32) -> Guard {
    note(f64::from(value));
    Guard(value)
}

#[inline(never)]
fn consume(_guard: Guard) {
    note(13.0);
}

pub async fn work<T>(case: u32, token: T) -> Result<T, JsValue> {
    let _outer = Guard(80);
    match case {
        0 => {
            let _inner = Guard(90);
            note(10.0);
        }
        1 => {
            await_promise(JsValue::undefined()).await?;
            let _inner = Guard(91);
            note(30.0);
        }
        2 => {
            let _pair = Pair {
                _first: Guard(81),
                _second: make_guard(82),
            };
            note(40.0);
        }
        3 => consume(Guard(83)),
        4 => {
            let mut guard = Guard(84);
            let _ = &guard;
            guard = make_guard(85);
            note(40.0);
            drop(guard);
        }
        _ => unreachable!(),
    }
    Ok(token)
}
