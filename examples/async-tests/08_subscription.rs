#![no_std]
#![no_main]
#![deny(unsafe_code)]

use js_extern_macro::{js_bindings, js_export};
use view_abi::JsValue;

js_bindings! {
    #[js(call = "subscriptionNote")]
    fn note(phase: f64);
    #[js(call = "subscriptionResult")]
    fn report_value(value: JsValue, ok: bool);
    #[js(call = "subscriptionResult")]
    fn report_string(value: &str, ok: bool);
    #[js(call = "subscriptionResult")]
    fn report_number(value: f64, ok: bool);
    #[js(call = "subscriptionUnit")]
    fn report_unit(ok: bool);
}

struct Guard(f64);

impl Drop for Guard {
    fn drop(&mut self) {
        note(self.0);
    }
}

async fn wait<V>(value: JsValue, captured: Guard) -> Result<V, V> {
    let _captured = captured;
    let _local = Guard(90.0);
    note(10.0);
    view_async::await_promise(value).await
}

fn spawn<V: 'static>(value: JsValue, report: impl Fn(V, bool) + 'static) {
    let captured = Guard(80.0);
    view_async::spawn(async move {
        match wait(value, captured).await {
            Ok(value) => report(value, true),
            Err(value) => report(value, false),
        }
        note(20.0);
    });
}

#[js_export]
pub fn start(value: JsValue, kind: u32) {
    note(1.0);
    match kind {
        0 => spawn(value, report_value),
        1 => spawn(value, report_string),
        2 => spawn(value, report_number),
        _ => spawn(value, |(): (), ok| report_unit(ok)),
    }
    note(2.0);
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}
