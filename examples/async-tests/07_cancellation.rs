#![no_std]
#![no_main]
#![deny(unsafe_code)]

use js_extern_macro::{js_bindings, js_export};
use view_abi::JsValue;

js_bindings! {
    #[js(call = "ownerNote")]
    fn note(task: f64, phase: f64);

    #[js(call = "ownerPending")]
    fn pending(task: f64, phase: f64) -> JsValue;

    #[js(call = "ownerResult")]
    fn result(task: f64, phase: f64, value: JsValue, ok: bool);

    #[js(call = "ownerAction")]
    fn action(task: f64, phase: f64);
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}

struct Guard {
    task: f64,
    phase: f64,
}

impl Drop for Guard {
    fn drop(&mut self) {
        note(self.task, self.phase);
    }
}

async fn body(task: f64, steps: u32, captured: Guard) {
    let _captured = captured;
    let _outer = Guard { task, phase: 90.0 };
    note(task, 10.0);
    for phase in 0..steps {
        let _inner = Guard {
            task,
            phase: 100.0 + f64::from(phase),
        };
        let value: Result<JsValue, JsValue> =
            view_async::await_promise(pending(task, f64::from(phase))).await;
        note(task, 20.0 + f64::from(phase));
        match value {
            Ok(value) => {
                result(task, f64::from(phase), value, true);
                action(task, f64::from(phase));
            }
            Err(value) => {
                result(task, f64::from(phase), value, false);
                note(task, 40.0);
                break;
            }
        }
    }
    note(task, 30.0);
}

#[js_export]
pub fn start(task: f64, steps: u32) {
    note(task, 1.0);
    view_async::spawn(body(task, steps, Guard { task, phase: 80.0 }));
    note(task, 2.0);
}
