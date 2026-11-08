//! An iterator source that has no proven materialized collection lane yet.

#![no_std]
#![no_main]

use view_dom_macro::dom_view_client_only;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[unsafe(no_mangle)]
pub fn rust_entry() -> view_abi::Node {
    view_abi::keyed_list_v1(
        || 1_u32..4,
        |item| item,
        |item, _index| {
            dom_view_client_only! {
                <button>(item)</button>
            }
        },
    )
}
