//! The accepted `for .. key (..)` syntax selecting the versioned keyed-list marker.

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

static ROWS: [Row; 3] = [
    Row { id: 1, label: 1 },
    Row { id: 2, label: 2 },
    Row { id: 3, label: 3 },
];

#[unsafe(no_mangle)]
pub fn rust_entry() -> view_abi::Node {
    dom_view_client_only! {
        <ul>
            for row in &ROWS key (row.id) {
                <li>$(row.label)</li>
            }
        </ul>
    }
}
