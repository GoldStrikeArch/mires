//! An owned product whose nested closure would capture a frozen field value.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use view_dom_macro::dom_view_client_only;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[derive(Clone, Copy)]
struct Row {
    id: u32,
    label: u32,
}

#[unsafe(no_mangle)]
pub fn rust_entry() -> view_abi::Node {
    view_abi::keyed_list_v1(
        || Vec::from([Row { id: 1, label: 10 }]),
        |row| row.id,
        |row, _index| {
            dom_view_client_only! {
                <button @click=$(move |_event| row.label)>$(row.label)</button>
            }
        },
    )
}
