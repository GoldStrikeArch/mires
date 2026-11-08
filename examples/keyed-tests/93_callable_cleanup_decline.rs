//! A dropful collection closure whose retained cleanup is not executable yet.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::boxed::Box;
use view_dom_macro::dom_view_client_only;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[unsafe(no_mangle)]
pub fn rust_entry() -> view_abi::Node {
    let start = Box::new(1_u32);
    view_abi::keyed_list_v1(
        move || [*start, 2, 3],
        |item| item,
        |item, _index| {
            dom_view_client_only! {
                <button>(item)</button>
            }
        },
    )
}
