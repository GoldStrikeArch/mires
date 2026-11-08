//! A retained-row reorder whose effect and handler both capture the live index.

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

static BEFORE: [Row; 3] = [Row { id: 1 }, Row { id: 2 }, Row { id: 3 }];
static AFTER: [Row; 3] = [Row { id: 3 }, Row { id: 1 }, Row { id: 2 }];

#[unsafe(no_mangle)]
pub fn rust_entry() -> view_abi::Node {
    let changed = view_abi::signal(0, false);
    let output = view_abi::list();
    let control = dom_view_client_only! {
        <button @click=$(move |_event| changed.set(true))>"update"</button>
    };
    view_abi::push(&output, control);
    view_abi::push(
        &output,
        view_abi::keyed_list_v1(
            || {
                if changed.get() {
                    &AFTER[..]
                } else {
                    &BEFORE[..]
                }
            },
            |row| row.id,
            |_row, index| {
                dom_view_client_only! {
                    <button @click=$(move |_event| index + 100)>$(index + 1)</button>
                }
            },
        ),
    );
    output
}
