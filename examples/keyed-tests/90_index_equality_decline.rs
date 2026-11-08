//! Negative control: JavaScript strict equality cannot observe a callable live-index proxy.

#![no_std]
#![no_main]

use view_dom_macro::dom_view_client_only;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

struct Row {
    id: u32,
}

static ROWS: [Row; 3] = [Row { id: 1 }, Row { id: 2 }, Row { id: 3 }];

#[unsafe(no_mangle)]
pub fn rust_entry() -> view_abi::Node {
    view_abi::keyed_list_v1(
        || &ROWS[..],
        |row| row.id,
        |_row, index| {
            dom_view_client_only! {
                <button @click=$(move |_event| index == 0)>$(index + 1)</button>
            }
        },
    )
}
