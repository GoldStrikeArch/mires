//! Removed keyed rows must be collectible while their island remains mounted.

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

static ROWS: [Row; 2] = [Row { id: 1, label: 10 }, Row { id: 2, label: 20 }];

#[no_mangle]
pub fn rust_entry() -> view_abi::Node {
    let visible = view_abi::signal(0, true);
    dom_view_client_only! {
        <section>
            <button @click=$(move |_event| visible.set(!visible.get()))>"toggle"</button>
            <ul>
                for row in { if visible.get() { &ROWS[..] } else { &[][..] } } key (row.id) {
                    <li>$(row.label)</li>
                }
            </ul>
        </section>
    }
}
