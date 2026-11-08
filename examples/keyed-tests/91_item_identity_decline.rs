//! Negative control: the callable live-item proxy is not the current Rust reference identity.

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
        |row, _index| {
            dom_view_client_only! {
                <button @click=$(move |_event| core::ptr::eq(row, &ROWS[0]))>$(row.id)</button>
            }
        },
    )
}
