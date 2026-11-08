//! A compiled static keyed list, used by `scripts/keyed-compiled-test.sh`.

#![no_std]
#![no_main]

use view_dom_macro::dom_view_client_only;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

fn keyed(rows: &[u32]) -> view_abi::Node {
    view_abi::keyed_list_v1(
        || rows,
        |row| *row,
        |row, _index| {
            dom_view_client_only! {
                <li>(*row)</li>
            }
        },
    )
}

#[unsafe(no_mangle)]
pub fn rust_entry() -> view_abi::Node {
    keyed(&[1, 2, 3])
}
