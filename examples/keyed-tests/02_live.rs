//! A live keyed row whose effect and stable handler both capture the row item.

#![no_std]
#![no_main]

use view_dom_macro::dom_view_client_only;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

struct Row {
    id: u32,
    label: u32,
}

fn keyed(rows: &[Row]) -> view_abi::Node {
    view_abi::keyed_list_v1(
        || rows,
        |row| row.id,
        |row, _index| {
            dom_view_client_only! {
                <button @click=$(|_event| row.label)>$(row.label)</button>
            }
        },
    )
}

#[unsafe(no_mangle)]
pub fn rust_entry() -> view_abi::Node {
    keyed(&[
        Row { id: 1, label: 1 },
        Row { id: 2, label: 2 },
        Row { id: 3, label: 3 },
    ])
}
